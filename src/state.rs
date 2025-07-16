use std::{path::PathBuf, vec};

use anyhow::{Context, Result};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use sqlx::{Row, SqlitePool, migrate::Migrator, sqlite::SqliteConnectOptions};
use zerocopy::IntoBytes;

use crate::{
    config::Config,
    task::{Task, TaskKind},
    tools::memory::{Entity, KnowledgeGraph, Observation, Relation},
    types::{AssistantContent, Message, Text, ToolCall, ToolResult, UserContent},
};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");
const VOYAGEAI_URL: &str = "https://api.voyageai.com/v1/embeddings";

#[derive(Debug, Clone)]
pub struct AgentState {
    pool: SqlitePool,
    has_new_tasks: bool,
    balance: u32,
    voyageai_api_key: SecretString,
    voyageai_model: String,
    voyageai_dimensions: u16,
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
            libsqlite3_sys::sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                unsafe extern "C" fn(
                    *mut libsqlite3_sys::sqlite3,
                    *mut *mut i8,
                    *const libsqlite3_sys::sqlite3_api_routines,
                ) -> i32,
            >(
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

        let res = sqlx::query("select vec_version()").fetch_one(&pool).await?;
        tracing::info!("vec_version={:?}", res.get::<String, _>(0));
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
            voyageai_dimensions: config.voyageai_dimensions,
            voyageai_http_client: None,
        })
    }

    async fn create_embedding(&mut self, text: &str) -> Result<Vec<f32>> {
        let client = self
            .voyageai_http_client
            .get_or_insert_with(reqwest::Client::new);
        let res = client
            .post(VOYAGEAI_URL)
            .header("Content-Type", "application/json")
            .header(
                "Authorization",
                format!("Bearer {}", self.voyageai_api_key.expose_secret()),
            )
            .json(&serde_json::json!({
                "model": self.voyageai_model,
                "output_dimension": self.voyageai_dimensions,
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

    #[allow(dead_code)]
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
                        person_id: t.social_id.clone().unwrap_or_default(),
                        social_id: t.social_id.unwrap_or_default(),
                        name: t.person_name.unwrap_or_default(),
                        content: t.content.unwrap_or_default(),
                    },
                    "mention" => TaskKind::Mention {
                        person_id: t.social_id.clone().unwrap_or_default(),
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
                    person_id: t.person_id.unwrap_or_default(),
                    social_id: t.social_id.unwrap_or_default(),
                    content: t.content.unwrap_or_default(),
                    name: t.person_name.unwrap_or_default(),
                },
                "mention" => TaskKind::Mention {
                    person_id: t.person_id.unwrap_or_default(),
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
        let (task_kind, social_id, person_id, name, channel_id, content) = match &task.kind {
            TaskKind::DirectQuestion {
                social_id,
                person_id,
                name,
                content,
            } => (
                "direct_question",
                Some(social_id),
                Some(person_id),
                Some(name),
                None,
                Some(content),
            ),
            TaskKind::Mention {
                social_id,
                person_id,
                name,
                channel_id,
                content,
                ..
            } => (
                "mention",
                Some(social_id),
                Some(person_id),
                Some(name),
                Some(channel_id),
                Some(content),
            ),
            TaskKind::FollowChat {
                channel_id,
                content,
            } => (
                "follow_chat",
                None,
                None,
                None,
                Some(channel_id),
                Some(content),
            ),
            TaskKind::Research => ("research", None, None, None, None, None),
            TaskKind::Sleep => ("sleep", None, None, None, None, None),
        };
        sqlx::query!(
            "INSERT INTO tasks (kind, social_id, person_id, person_name, channel_id, content) VALUES (?, ?, ?, ?, ?, ?)",
            task_kind,
            social_id,
            person_id,
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
    async fn entity_id_by_name(&self, name: &str) -> Result<i64> {
        sqlx::query!("SELECT id FROM mem_entity WHERE name = ?", name)
            .fetch_one(&self.pool)
            .await?
            .id
            .ok_or_else(|| anyhow::anyhow!("Entity {} not found", name))
    }

    async fn entity_observations(&self, entity_id: i64) -> Result<Vec<String>> {
        let observations = sqlx::query!(
            "SELECT observation FROM mem_observation WHERE entity_id = ?",
            entity_id
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|o| o.observation)
        .collect::<Vec<_>>();
        Ok(observations)
    }

    async fn relations_by_entity(&self, entity_id: i64) -> Result<Vec<Relation>> {
        let relations = sqlx::query!(
            "SELECT
                        en1.name as name_from,
                        en2.name name_to,
                        rel.relation_type
                    FROM mem_relation as rel,
                         mem_entity as en1,
                         mem_entity as en2
                    WHERE (rel.from_id = ? OR rel.to_id = ?)
                      AND rel.from_id = en1.id
                      AND rel.to_id = en2.id",
            entity_id,
            entity_id
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|r| Relation {
            from: r.name_from,
            to: r.name_to,
            relation_type: r.relation_type,
        })
        .collect();
        Ok(relations)
    }

    pub async fn mem_list_relevant_entities(&mut self, query: &str) -> Result<Vec<Entity>> {
        let query_embedding = self
            .create_embedding(query)
            .await
            .with_context(|| "Failed to create embedding")?;
        let query_embedding = query_embedding.as_bytes();
        let mut entries = sqlx::query("SELECT id, name, type FROM mem_entity WHERE id IN (SELECT rowid FROM vec_mem_entity WHERE embedding MATCH ? ORDER BY distance LIMIT ?)")
        .bind(query_embedding)
        .bind(10)
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|row| Entity {
                id: row.get::<i64, _>(0),
                name: row.get::<String, _>(1),
                entity_type: row.get::<String, _>(2),
                observations: Vec::new(),
            }).collect::<Vec<_>>();
        for entry in &mut entries {
            entry.observations = self.entity_observations(entry.id).await?;
        }
        Ok(entries)
    }

    pub async fn mem_add_entities(&mut self, entities: &mut [Entity]) -> Result<Vec<Entity>> {
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
            let mut tx = self.pool.begin().await?;
            let row_id = sqlx::query!(
                "INSERT INTO mem_entity (name, type) VALUES (?, ?)",
                entity.name,
                entity.entity_type
            )
            .execute(&mut *tx)
            .await?
            .last_insert_rowid();
            for observation in &entity.observations {
                sqlx::query!(
                    "INSERT INTO mem_observation (entity_id, observation) VALUES (?, ?)",
                    row_id,
                    observation
                )
                .execute(&mut *tx)
                .await?;
            }
            sqlx::query("INSERT INTO vec_mem_entity (rowid, embedding) VALUES (?, ?)")
                .bind(row_id)
                .bind(embedding)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            entities_to_add.push(entity.clone());
        }
        Ok(entities_to_add)
    }

    pub async fn mem_add_relations(&mut self, relations: &mut [Relation]) -> Result<Vec<Relation>> {
        let mut relations_to_add = Vec::new();
        for relation in relations.iter_mut() {
            let from_id = self.entity_id_by_name(&relation.from).await?;
            let to_id = self.entity_id_by_name(&relation.to).await?;

            if sqlx::query!(
                "SELECT id FROM mem_relation WHERE relation_type = ? AND from_id = ? AND to_id = ?",
                from_id,
                to_id,
                relation.relation_type
            )
            .fetch_optional(&self.pool)
            .await?
            .is_some()
            {
                continue;
            }

            sqlx::query!(
                "INSERT INTO mem_relation (from_id, to_id, relation_type) VALUES (?, ?, ?)",
                from_id,
                to_id,
                relation.relation_type
            )
            .execute(&self.pool)
            .await?;
            relations_to_add.push(relation.clone());
        }
        Ok(relations_to_add)
    }

    pub async fn mem_add_observations(
        &mut self,
        observations: Vec<Observation>,
    ) -> Result<Vec<Observation>> {
        let mut observations_to_add = Vec::new();
        for observation in observations.into_iter() {
            let entity_id = self.entity_id_by_name(&observation.entity_name).await?;

            let mut new_observation = Observation {
                entity_name: observation.entity_name,
                observations: Vec::new(),
            };
            for observation_item in observation.observations {
                if sqlx::query!(
                    "SELECT * FROM mem_observation WHERE entity_id = ? AND observation = ?",
                    entity_id,
                    observation_item
                )
                .fetch_optional(&self.pool)
                .await?
                .is_some()
                {
                    continue;
                }
                sqlx::query!(
                    "INSERT INTO mem_observation (entity_id, observation) VALUES (?, ?)",
                    entity_id,
                    observation_item
                )
                .execute(&self.pool)
                .await?;
                new_observation.observations.push(observation_item);
            }
            observations_to_add.push(new_observation);
        }
        Ok(observations_to_add)
    }

    pub async fn mem_delete_entities(&mut self, entity_names: &[String]) -> Result<()> {
        for entity_name in entity_names {
            let entity_id = self.entity_id_by_name(entity_name).await?;

            let mut tx = self.pool.begin().await?;
            sqlx::query!("DELETE FROM mem_observation WHERE entity_id = ?", entity_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query!(
                "DELETE FROM mem_relation WHERE from_id = ? OR to_id = ?",
                entity_id,
                entity_id
            )
            .execute(&mut *tx)
            .await?;
            sqlx::query!("DELETE FROM mem_entity WHERE id = ?", entity_id)
                .execute(&mut *tx)
                .await?;

            tx.commit().await?;
        }
        Ok(())
    }

    pub async fn mem_delete_observations(&mut self, observations: &[Observation]) -> Result<()> {
        for observation in observations {
            let entity_id = self.entity_id_by_name(&observation.entity_name).await?;
            for observation_item in &observation.observations {
                sqlx::query!(
                    "DELETE FROM mem_observation WHERE entity_id = ? AND observation = ?",
                    entity_id,
                    observation_item
                )
                .execute(&self.pool)
                .await?;
            }
        }
        Ok(())
    }

    pub async fn mem_delete_relations(&mut self, relations: &[Relation]) -> Result<()> {
        for relation in relations {
            let from_id = self.entity_id_by_name(&relation.from).await?;
            let to_id = self.entity_id_by_name(&relation.to).await?;

            sqlx::query!(
                "DELETE FROM mem_relation WHERE from_id = ? AND to_id = ?",
                from_id,
                to_id
            )
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    pub async fn mem_search_nodes(&self, query: Option<&str>) -> Result<KnowledgeGraph> {
        if let Some(query) = query {
            let query = format!("%{query}%");
            let mut entities =
                sqlx::query!("SELECT * FROM mem_entity WHERE name LIKE ? OR type LIKE ? ORDER BY created_at DESC LIMIT 100", query, query)
                .fetch_all(&self.pool).await?
                .into_iter().map(|r| Entity { id: r.id.unwrap_or_default(), name: r.name, entity_type: r.r#type, observations: vec![] })
                .collect::<Vec<_>>();
            let mut relations = vec![];
            for entity in &mut entities {
                entity.observations = self.entity_observations(entity.id).await?;
                relations.extend(self.relations_by_entity(entity.id).await?);
            }
            Ok(KnowledgeGraph {
                entities,
                relations,
            })
        } else {
            let mut entities =
                sqlx::query!("SELECT * FROM mem_entity ORDER BY created_at DESC LIMIT 100")
                    .fetch_all(&self.pool)
                    .await?
                    .into_iter()
                    .map(|r| Entity {
                        id: r.id.unwrap_or_default(),
                        name: r.name,
                        entity_type: r.r#type,
                        observations: vec![],
                    })
                    .collect::<Vec<_>>();
            let mut relations = vec![];
            for entity in &mut entities {
                entity.observations = self.entity_observations(entity.id).await?;
                relations.extend(self.relations_by_entity(entity.id).await?);
            }
            Ok(KnowledgeGraph {
                entities,
                relations,
            })
        }
    }

    pub async fn mem_list_entities(&self, names: &[String]) -> Result<Vec<Entity>> {
        let mut entities = Vec::new();
        for name in names {
            let entity = sqlx::query!("SELECT * FROM mem_entity WHERE name = ?", name)
                .fetch_optional(&self.pool)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Entity {} not found", name))?;
            entities.push(Entity {
                id: entity.id.unwrap_or_default(),
                name: entity.name,
                entity_type: entity.r#type,
                observations: self
                    .entity_observations(entity.id.unwrap_or_default())
                    .await?,
            });
        }
        Ok(entities)
    }
    // #endregion
}
