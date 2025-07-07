use std::path::PathBuf;

use anyhow::Result;
use sqlx::{migrate::Migrator, sqlite::SqliteConnectOptions, SqlitePool};

use crate::{
    task::{Task, TaskKind},
    types::{AssistantContent, Message, Text, ToolCall, ToolResult, UserContent},
};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

#[derive(Debug, Clone)]
pub struct AgentState {
    pool: SqlitePool,
    has_new_tasks: bool,
    balance: u32,
}

fn trace_message(message: &Message) {
    match message {
        Message::User { content } => {
            let msg = match content.first().unwrap() {
                UserContent::Text(Text { text }) => text,
                UserContent::ToolResult(ToolResult { id, .. }) => &format!("⚙️ {id}"),
                _ => "unknown",
            };

            tracing::info!(log_message = true, "👨‍: {}", msg);
        }
        Message::Assistant { content } => {
            let msg = content
                .iter()
                .map(|c| match c {
                    AssistantContent::Text(Text { text }) => text.to_string(),
                    AssistantContent::ToolCall(ToolCall { function, .. }) => {
                        format!("⚙️ {}", function.name)
                    }
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            tracing::info!(log_message = true, "🤖: {}", msg);
        }
    }
}

impl AgentState {
    pub async fn new(data_dir: PathBuf) -> Result<Self> {
        let opt = SqliteConnectOptions::new()
            .create_if_missing(true)
            .filename(format!(
                "file:{}",
                data_dir.join("state.db").to_str().unwrap()
            ));
        let pool = SqlitePool::connect_with(opt).await?;
        MIGRATOR.run(&pool).await?;
        let balance = sqlx::query!("SELECT balance FROM agent_state")
            .fetch_one(&pool)
            .await?;
        Ok(Self {
            pool,
            balance: balance.balance.try_into().unwrap_or_default(),
            has_new_tasks: true,
        })
    }

    pub async fn tasks(&self) -> Result<Vec<Task>> {
        let tasks = sqlx::query!("SELECT * FROM tasks WHERE is_done = 0 order by created_at desc");
        let tasks = tasks
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|t| Task {
                id: t.id,
                kind: match t.kind.as_str() {
                    "direct_question" => TaskKind::DirectQuestion {
                        social_id: t.social_id.unwrap_or_default(),
                        name: t.person_name.unwrap_or_default(),
                        content: t.content.unwrap_or_default(),
                    },
                    "mention" => TaskKind::Mention {
                        social_id: t.social_id.unwrap_or_default(),
                        name: t.person_name.unwrap_or_default(),
                        channel_id: t.channel_id.unwrap_or_default(),
                        content: t.content.unwrap_or_default(),
                    },
                    "follow_chat" => TaskKind::FollowChat {
                        channel_id: t.channel_id.unwrap_or_default(),
                        content: t.content.unwrap_or_default(),
                    },
                    "research" => TaskKind::Research,
                    "sleep" => TaskKind::Sleep,
                    _ => unreachable!(),
                },
                created_at: t.created_at.and_utc(),
                updated_at: t.updated_at.and_utc(),
            })
            .collect();
        Ok(tasks)
    }

    pub async fn latest_task(&mut self) -> Result<Option<Task>> {
        if !self.has_new_tasks {
            return Ok(None);
        }
        let task =
            sqlx::query!("SELECT * FROM tasks WHERE is_done = 0 order by created_at desc limit 1");
        let task = task.fetch_optional(&self.pool).await?.map(|t| Task {
            id: t.id,
            kind: match t.kind.as_str() {
                "direct_question" => TaskKind::DirectQuestion {
                    social_id: t.social_id.unwrap_or_default(),
                    content: t.content.unwrap_or_default(),
                    name: t.person_name.unwrap_or_default(),
                },
                "mention" => TaskKind::Mention {
                    social_id: t.social_id.unwrap_or_default(),
                    name: t.person_name.unwrap_or_default(),
                    channel_id: t.channel_id.unwrap_or_default(),
                    content: t.content.unwrap_or_default(),
                },
                "follow_chat" => TaskKind::FollowChat {
                    channel_id: t.channel_id.unwrap_or_default(),
                    content: t.content.unwrap_or_default(),
                },
                "research" => TaskKind::Research,
                "sleep" => TaskKind::Sleep,
                _ => unreachable!(),
            },
            created_at: t.created_at.and_utc(),
            updated_at: t.updated_at.and_utc(),
        });
        if task.is_none() {
            self.has_new_tasks = false;
        }
        Ok(task)
    }

    pub async fn task_messages(&self, task_id: i64) -> Result<Vec<Message>> {
        let messages = sqlx::query!("SELECT * FROM task_message WHERE task_id = ?", task_id);
        let messages = messages
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|m| serde_json::from_str(&m.content).unwrap())
            .collect();
        Ok(messages)
    }

    pub fn balance(&self) -> u32 {
        self.balance
    }

    pub async fn set_balance(&mut self, balance: u32) -> Result<()> {
        sqlx::query!("UPDATE agent_state SET balance = ?", balance)
            .execute(&self.pool)
            .await?;
        self.balance = balance;
        Ok(())
    }

    pub async fn add_task(&mut self, task: Task) -> Result<()> {
        let (task_kind, social_id, name, channel_id, content) = match &task.kind {
            TaskKind::DirectQuestion {
                social_id,
                name,
                content,
            } => (
                "direct_question",
                Some(social_id),
                Some(name),
                None,
                Some(content),
            ),
            TaskKind::Mention {
                social_id,
                name,
                channel_id,
                content,
            } => (
                "mention",
                Some(social_id),
                Some(name),
                Some(channel_id),
                Some(content),
            ),
            TaskKind::FollowChat {
                channel_id,
                content,
            } => ("follow_chat", None, None, Some(channel_id), Some(content)),
            TaskKind::Research => ("research", None, None, None, None),
            TaskKind::Sleep => ("sleep", None, None, None, None),
        };
        sqlx::query!(
            "INSERT INTO tasks (kind, social_id, person_name, channel_id, content) VALUES (?, ?, ?, ?, ?)",
            task_kind,
            social_id,
            name,
            channel_id,
            content
        )
        .execute(&self.pool)
        .await?;
        self.has_new_tasks = true;
        Ok(())
    }

    pub async fn add_task_message(&mut self, task: &mut Task, message: Message) -> Result<Message> {
        trace_message(&message);
        let json_message = serde_json::to_string(&message)?;
        sqlx::query!(
            "INSERT INTO task_message (task_id, content) VALUES (?, ?)",
            task.id,
            json_message
        )
        .execute(&self.pool)
        .await?;
        Ok(message)
    }

    pub async fn set_task_done(&mut self, task_id: i64) -> Result<()> {
        sqlx::query!("UPDATE tasks SET is_done = 1 WHERE id = ?", task_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
