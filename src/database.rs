// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use secrecy::ExposeSecret;
use serde::Deserialize;
use sqlx::{Row, SqliteConnection};
use std::path::Path;
use zerocopy::IntoBytes;

use crate::memory::MemoryEntityType;
use crate::{
    config::Config,
    memory::MemoryEntity,
    task::{Task, TaskKind},
    types::Message,
};
use sqlx::{SqlitePool, migrate::Migrator, sqlite::SqliteConnectOptions};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");
const VOYAGEAI_URL: &str = "https://api.voyageai.com/v1/embeddings";

#[derive(Debug, Deserialize)]
struct VoyageAIEmbeddingResponse {
    pub data: Vec<VoyageAIEmbedding>,
}

#[derive(Debug, Deserialize)]
struct VoyageAIEmbedding {
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct DbClient {
    pool: SqlitePool,
    voyageai_model: String,
    voyageai_dimensions: u16,
    voyageai_http_client: reqwest::Client,
}

macro_rules! to_task {
    ($record:expr) => {
        Task {
            id: $record.id.unwrap_or_default(),
            kind: match $record.kind.as_str() {
                "follow_chat" => TaskKind::FollowChat {
                    channel_id: $record.channel_id.unwrap_or_default(),
                    channel_title: $record.channel_title.unwrap_or_default(),
                    content: $record.content.unwrap_or_default(),
                },
                "memory_mantainance" => TaskKind::MemoryMantainance,
                "sleep" => TaskKind::Sleep,
                _ => unreachable!(),
            },
            created_at: $record.created_at.and_utc(),
            updated_at: $record.updated_at.and_utc(),
        }
    };
}

macro_rules! to_mem_entity {
    ($record:expr) => {
        MemoryEntity {
            id: $record.id.unwrap_or_default(),
            name: $record.name,
            category: $record.category,
            entity_type: MemoryEntityType::from_i64($record.entity_type),
            importance: $record.importance as f32,
            access_count: $record.access_count as u32,
            relations: vec![],
            observations: serde_json::from_str(&$record.observations).unwrap_or_default(),
            created_at: $record.created_at.and_utc(),
            updated_at: $record.updated_at.and_utc(),
        }
    };
}

impl DbClient {
    pub async fn new(data_dir: &str, config: &Config) -> Result<Self> {
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
                Path::new(data_dir)
                    .to_path_buf()
                    .join("state.db")
                    .to_str()
                    .unwrap()
            ));
        let pool = SqlitePool::connect_with(opt).await?;
        let res = sqlx::query("select vec_version()").fetch_one(&pool).await?;
        tracing::info!("vec_version={:?}", res.get::<String, _>(0));
        MIGRATOR.run(&pool).await?;

        Ok(Self {
            pool,
            voyageai_model: config.voyageai_model.clone(),
            voyageai_dimensions: config.voyageai_dimensions,
            voyageai_http_client: reqwest::ClientBuilder::new()
                .default_headers({
                    let mut headers = reqwest::header::HeaderMap::new();
                    headers.insert(
                        "Content-Type",
                        reqwest::header::HeaderValue::from_static("application/json"),
                    );
                    headers.insert(
                        "Authorization",
                        format!("Bearer {}", config.voyageai_api_key.expose_secret()).parse()?,
                    );
                    headers
                })
                .build()?,
        })
    }

    async fn create_embedding(&self, text: &str) -> Result<Vec<f32>> {
        let res = self
            .voyageai_http_client
            .post(VOYAGEAI_URL)
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

    pub async fn balance(&self) -> Result<u32, sqlx::Error> {
        let balance = sqlx::query!("SELECT balance FROM agent_state")
            .fetch_one(&self.pool)
            .await?;
        Ok(balance.balance.try_into().unwrap_or_default())
    }

    pub async fn set_balance(&mut self, balance: u32) -> Result<()> {
        sqlx::query!("UPDATE agent_state SET balance = ?", balance)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn tasks(&self) -> Result<Vec<Task>> {
        let tasks = sqlx::query!("SELECT * FROM tasks WHERE is_done = 0 order by created_at desc");
        let tasks = tasks
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|record| to_task!(record))
            .collect();
        Ok(tasks)
    }

    pub async fn latest_task(&self) -> Result<Option<Task>> {
        let task =
            sqlx::query!("SELECT * FROM tasks WHERE is_done = 0 order by created_at desc limit 1");
        Ok(task
            .fetch_optional(&self.pool)
            .await?
            .map(|record| to_task!(record)))
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

    pub async fn add_task(&mut self, task: Task) -> Result<()> {
        let (task_kind, social_id, person_id, name, channel_id, channel_title, content, message_id) =
            match &task.kind {
                TaskKind::FollowChat {
                    channel_id,
                    channel_title,
                    content,
                } => (
                    "follow_chat",
                    None::<String>,
                    None::<String>,
                    None::<String>,
                    Some(channel_id),
                    Some(channel_title),
                    Some(content),
                    None::<String>,
                ),
                TaskKind::MemoryMantainance => (
                    "memory_mantainance",
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                ),
                TaskKind::Sleep => ("sleep", None, None, None, None, None, None, None),
            };
        sqlx::query!(
            "INSERT INTO tasks (kind, social_id, person_id, person_name, channel_id, channel_title, content, message_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            task_kind,
            social_id,
            person_id,
            name,
            channel_id,
            channel_title,
            content,
            message_id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn add_task_message(&mut self, task: &mut Task, message: Message) -> Result<Message> {
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

    pub async fn delete_old_tasks(&self, expire_date: DateTime<Utc>) -> Result<()> {
        tracing::info!(%expire_date, "Delete old tasks");
        let mut tx = self.pool.begin().await?;
        sqlx::query!(
            "DELETE FROM task_message WHERE task_id IN (SELECT id FROM tasks WHERE is_done = 1 AND updated_at < ?)",
            expire_date
        )
        .execute(&mut *tx)
        .await?;
        let count = sqlx::query!(
            "DELETE FROM tasks WHERE is_done = 1 AND updated_at < ?",
            expire_date
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        tracing::info!(%count, "Deleted tasks");
        sqlx::query!("VACUUM").execute(&self.pool).await?;
        Ok(())
    }

    //#region memory
    async fn create_entity_embedding(&self, entity: &MemoryEntity) -> Result<Vec<f32>> {
        let text_for_embedding = format!(
            r#"Entity name: {}\n
               Category: {}\n
               Observations: {}\n"#,
            entity.name,
            entity.category,
            entity.observations.join("\n")
        );

        self.create_embedding(&text_for_embedding)
            .await
            .with_context(|| "Failed to create embedding")
    }

    async fn relations_by_entity(&self, entity_id: i64, entity_name: &str) -> Vec<String> {
        if let Ok(relations) = sqlx::query!(
            r#"
            SELECT
                en1.name as name_from,
                en2.name as name_to
            FROM mem_relation as rel,
                    mem_entity as en1,
                    mem_entity as en2
            WHERE (rel.from_id = ? OR rel.to_id = ?)
                AND rel.from_id = en1.id
                AND rel.to_id = en2.id
            "#,
            entity_id,
            entity_id
        )
        .fetch_all(&self.pool)
        .await
        {
            relations
                .into_iter()
                .map(|r| {
                    if r.name_from == entity_name {
                        r.name_to
                    } else {
                        r.name_from
                    }
                })
                .collect()
        } else {
            vec![]
        }
    }

    async fn mem_update_relations(
        &self,
        tx: &mut SqliteConnection,
        from_id: i64,
        relations: &[String],
    ) -> Result<()> {
        // clear all relations
        sqlx::query!(
            "DELETE FROM mem_relation WHERE from_id = ? OR to_id = ?",
            from_id,
            from_id
        )
        .execute(&mut *tx)
        .await?;

        for relation in relations {
            let Some(to_id) = sqlx::query!(
                "SELECT id FROM mem_entity WHERE lower(name) = lower(?)",
                relation
            )
            .fetch_optional(&mut *tx)
            .await?
            .and_then(|r| r.id) else {
                continue;
            };
            sqlx::query("INSERT INTO mem_relation (from_id, to_id) VALUES (?, ?)")
                .bind(from_id)
                .bind(to_id)
                .execute(&mut *tx)
                .await?;
        }

        Ok(())
    }

    /// Get entity by name and use lower case name representation
    pub async fn mem_entity_by_name(
        &self,
        name: &str,
        entity_type: MemoryEntityType,
    ) -> Option<MemoryEntity> {
        let mut entity = sqlx::query!(
            "SELECT * FROM mem_entity WHERE lower(name) = lower(?) and entity_type = ?",
            name,
            entity_type
        )
        .fetch_one(&self.pool)
        .await
        .map(|record| to_mem_entity!(record))
        .ok()?;
        entity.relations = self.relations_by_entity(entity.id, &entity.name).await;
        Some(entity)
    }

    pub async fn mem_entity(&self, id: i64) -> Result<MemoryEntity> {
        let record = sqlx::query!("SELECT * FROM mem_entity WHERE id = ?", id)
            .fetch_one(&self.pool)
            .await?;
        let mut entity = to_mem_entity!(record);
        entity.relations = self.relations_by_entity(id, &entity.name).await;
        Ok(entity)
    }

    pub async fn mem_update_entity(&self, entity: &MemoryEntity) -> Result<()> {
        let observations = serde_json::to_string(&entity.observations).unwrap();

        let embedding = self.create_entity_embedding(entity).await?;
        let mut tx = self.pool.begin().await?;

        let row_id = sqlx::query!("SELECT rowid FROM mem_entity WHERE id = ?", entity.id)
            .fetch_one(&mut *tx)
            .await?
            .id;
        sqlx::query!(
            "UPDATE mem_entity SET name = ?, entity_type = ?, category = ?, importance = ?, access_count = ?, observations = ?, updated_at = ? WHERE id = ?",
            entity.name,
            entity.entity_type,
            entity.category,
            entity.importance,
            entity.access_count,
            observations,
            entity.updated_at,
            entity.id
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query("UPDATE vec_mem_entity1 SET entity_type = ?, embedding = ? WHERE rowid = ?")
            .bind(entity.entity_type.clone())
            .bind(embedding.as_bytes())
            .bind(row_id)
            .execute(&mut *tx)
            .await?;

        self.mem_update_relations(&mut tx, entity.id, &entity.relations)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn mem_update_entity_importance(&self, id: i64, importance: f32) -> Result<()> {
        sqlx::query!(
            "UPDATE mem_entity SET importance = ? WHERE id = ?",
            importance,
            id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mem_add_entity(&self, entity: &MemoryEntity) -> Result<()> {
        let observations = serde_json::to_string(&entity.observations).unwrap();

        let embedding = self.create_entity_embedding(entity).await?;
        let mut tx = self.pool.begin().await?;

        let row_id =sqlx::query!(
            "INSERT INTO mem_entity (name, entity_type, category, importance, access_count, observations) VALUES (?, ?, ?, ?, ?, ?)",
            entity.name,
            entity.entity_type,
            entity.category,
            entity.importance,
            entity.access_count,
            observations,
        )
        .execute(&mut *tx)
        .await?
        .last_insert_rowid();

        sqlx::query("INSERT INTO vec_mem_entity1 (rowid, entity_type, embedding) VALUES (?, ?, ?)")
            .bind(row_id)
            .bind(entity.entity_type.clone())
            .bind(embedding.as_bytes())
            .execute(&mut *tx)
            .await?;

        let id = sqlx::query!("SELECT id FROM mem_entity WHERE rowid = ?", row_id)
            .fetch_one(&mut *tx)
            .await?
            .id
            .unwrap();
        self.mem_update_relations(&mut tx, id, &entity.relations)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn mem_last_entities(&self, limit: u16) -> Result<Vec<MemoryEntity>> {
        let mut entities = sqlx::query!(
            "SELECT * FROM mem_entity ORDER BY importance DESC, updated_at DESC LIMIT ?",
            limit
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|record| to_mem_entity!(record))
        .collect::<Vec<MemoryEntity>>();
        for entity in entities.iter_mut() {
            entity.relations = self.relations_by_entity(entity.id, &entity.name).await;
        }
        Ok(entities)
    }

    pub async fn mem_entities_ids_for_consolidation(&self, threshold: f32) -> Result<Vec<i64>> {
        let ids =sqlx::query!(
            "SELECT id FROM mem_entity WHERE importance >= ? AND entity_type == 0 ORDER BY updated_at DESC LIMIT 10000",
            threshold,
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .filter_map(|record| record.id)
        .collect::<Vec<_>>();
        Ok(ids)
    }

    pub async fn mem_relevant_entities(
        &self,
        limit: u16,
        query: &str,
        entity_type: MemoryEntityType,
    ) -> Result<Vec<MemoryEntity>> {
        let query_embedding = self
            .create_embedding(query)
            .await
            .with_context(|| "Failed to create embedding")?;
        let query_embedding = query_embedding.as_bytes();
        let mut entries = sqlx::query(
            r#"
                WITH matches as (
                    SELECT rowid, distance FROM vec_mem_entity1
                    WHERE entity_type = ? AND embedding MATCH ?
                    ORDER BY distance
                    LIMIT ?
                )
                SELECT * FROM mem_entity
                JOIN matches on mem_entity.rowid = matches.rowid
                ORDER BY distance ASC, importance DESC, updated_at DESC
            "#,
        )
        .bind(entity_type)
        .bind(query_embedding)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|record| MemoryEntity {
            id: record.get("id"),
            name: record.get("name"),
            entity_type: record.get("entity_type"),
            category: record.get("category"),
            importance: record.get("importance"),
            access_count: record.get("access_count"),
            relations: vec![],
            observations: serde_json::from_str(&record.get::<String, _>("observations"))
                .unwrap_or_default(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .collect::<Vec<MemoryEntity>>();
        for entity in entries.iter_mut() {
            entity.relations = self.relations_by_entity(entity.id, &entity.name).await;
        }
        Ok(entries)
    }

    pub async fn mem_get_entity_ids(&self) -> Result<Vec<i64>> {
        let idxs = sqlx::query!("SELECT id FROM mem_entity")
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .filter_map(|record| record.id)
            .collect::<Vec<i64>>();
        Ok(idxs)
    }

    pub async fn mem_delete_entity(&self, id: i64) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query!(
            "DELETE FROM mem_relation WHERE from_id = ? OR to_id = ?",
            id,
            id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!("DELETE FROM mem_entity WHERE id = ?", id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    //#endregion
}
