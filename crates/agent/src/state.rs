use std::path::PathBuf;

use anyhow::{Context, Result};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use sqlx::{Row, SqlitePool, migrate::Migrator, sqlite::SqliteConnectOptions};
use zerocopy::IntoBytes;

use crate::{
    config::Config,
    task::{Task, TaskKind},
    tools::memory::Entity,
    types::{AssistantContent, Message, Text, ToolCall, ToolResult, UserContent},
};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");
const VOYAGEAI_URL: &str = "https://api.voyageai.com/v1/embeddings";

#[derive(Debug, Clone)]
pub struct AgentState {
    pool: SqlitePool,
    has_new_tasks: bool,
    balance: u32,
    voyageai_api_key: Option<SecretString>,
    voyageai_model: Option<String>,
    voyageai_dimensions: Option<u16>,
    voyageai_http_client: Option<reqwest::Client>,
}

#[derive(Debug, Deserialize)]
struct VoyageAIEmbeddingResponse {
    pub data: Vec<VoyageAIEmbedding>,
}

#[derive(Debug, Deserialize)]
struct VoyageAIEmbedding {
    pub embedding: Vec<f32>,
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
    pub async fn new(data_dir: PathBuf, config: &Config) -> Result<Self> {
        unsafe {
            libsqlite3_sys::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }

        let opt = SqliteConnectOptions::new()
            .create_if_missing(true)
            .filename(format!(
                "file:{}",
                data_dir.join("state.db").to_str().unwrap()
            ));
        let pool = SqlitePool::connect_with(opt).await?;

        let res = sqlx::query!("select * from pragma_function_list")
            .fetch_one(&pool)
            .await?;
        // let res = res.columns();
        // println!("{:?}", res);
        MIGRATOR.run(&pool).await?;
        let balance = sqlx::query!("SELECT balance FROM agent_state")
            .fetch_one(&pool)
            .await?;
        Ok(Self {
            pool,
            balance: balance.balance.try_into().unwrap_or_default(),
            has_new_tasks: true,
            voyageai_api_key: config.voyageai_api_key.clone(),
            voyageai_model: config.voyageai_model.clone(),
            voyageai_http_client: None,
        })
    }

    async fn create_embedding(&mut self, text: &str) -> Result<Vec<f32>> {
        let client = self
            .voyageai_http_client
            .get_or_insert_with(|| reqwest::Client::new());
        let res = client
            .post(VOYAGEAI_URL)
            .header("Content-Type", "application/json")
            .header(
                "Authorization",
                format!(
                    "Bearer {}",
                    self.voyageai_api_key.as_ref().unwrap().expose_secret()
                ),
            )
            .json(&serde_json::json!({
                "model": self.voyageai_model.as_ref().unwrap(),
                "input": text,
            }))
            .send()
            .await?;
        let mut res = res.json::<VoyageAIEmbeddingResponse>().await?;

        let Some(embedding) = res.data.drain(..).next() else {
            anyhow::bail!("No embedding generated");
        };

        Ok(embedding.embedding)
    }

    pub async fn tasks(&self) -> Result<Vec<Task>> {
        let tasks = sqlx::query!("SELECT * FROM tasks WHERE is_done = 0 order by created_at desc");
        let tasks = tasks
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|t| Task {
                id: t.id.unwrap_or_default(),
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
            id: t.id.unwrap_or_default(),
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

    // #region memory
    pub async fn mem_add_entities(&mut self, entities: &mut Vec<Entity>) -> Result<Vec<Entity>> {
        let mut entities_to_add = Vec::new();
        for entity in entities.iter_mut() {
            if sqlx::query!("SELECT * FROM mem_entity WHERE name = ?", entity.name)
                .fetch_optional(&self.pool)
                .await?
                .is_some()
            {
                continue;
            }
            let text_for_embedding = format!(
                r#"Entity name: {}\n
                   Entity type: {}\n
                   Observations: {}\n"#,
                entity.name,
                entity.entity_type,
                entity.observations.join("\n")
            );

            let embedding = self
                .create_embedding(&text_for_embedding)
                .await
                .with_context(|| "Failed to create embedding")?;
            let embedding = embedding.as_bytes();
            let row_id = sqlx::query!(
                "INSERT INTO mem_entity (name, type, embedding) VALUES (?, ?, ?)",
                entity.name,
                entity.entity_type,
                embedding
            )
            .execute(&self.pool)
            .await?
            .last_insert_rowid();
            sqlx::query("INSERT INTO vec_mem_entity (rowid, embedding) VALUES (?, ?)")
                .bind(row_id)
                .bind(embedding)
                .execute(&self.pool)
                .await?;
            entities_to_add.push(entity.clone());
        }
        Ok(entities_to_add)
    }
    // #endregion
}
