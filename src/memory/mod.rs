// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{collections::HashMap, fmt::Display, vec};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::{Client, ClientBuilder};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{sync::mpsc::UnboundedReceiver, task::JoinHandle};

mod importance;

use crate::{
    config::Config,
    memory::importance::ImportanceCalculator,
    task::{Task, TaskKind},
};

const MAX_OBSERVATIONS: usize = 20;
const MAX_MEMORY_ENTITIES: u16 = 10;
const DELETE_THRESHOLD: f32 = 0.01;

const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
struct MemoryExtractor {
    client: Client,
    system_prompt: String,
    model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExtractedMemoryEntity {
    pub entity_name: String,
    pub category: String,
    pub observations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntity {
    #[serde(skip)]
    pub id: i64,
    pub name: String,
    pub category: String,
    #[serde(rename = "type")]
    pub entity_type: MemoryEntityType,
    #[serde(skip)]
    pub importance: f32,
    #[serde(skip)]
    pub access_count: u32,
    pub observations: Vec<String>,
    pub relations: Vec<String>,
    #[allow(dead_code)]
    #[serde(skip)]
    pub created_at: DateTime<Utc>,
    #[serde(skip_deserializing)]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::Type, Serialize, Deserialize)]
#[repr(i32)]
#[serde(rename_all = "lowercase")]
pub enum MemoryEntityType {
    Episode = 0,
    Semantic = 1,
}

impl MemoryEntityType {
    pub fn from_i64(value: i64) -> Self {
        match value {
            0 => MemoryEntityType::Episode,
            1 => MemoryEntityType::Semantic,
            _ => MemoryEntityType::Episode,
        }
    }
}

impl Display for MemoryEntityType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryEntityType::Episode => write!(f, "episode"),
            MemoryEntityType::Semantic => write!(f, "semantic"),
        }
    }
}

impl MemoryEntity {
    pub fn format(&self) -> String {
        format!(
            "### {}\n\nType: {}\n\nCategory: {}\n\nImportance: {}\n\nRelations:\n{}\n\nObservations:\n{}\n",
            self.name,
            self.entity_type,
            self.category,
            self.importance,
            self.relations
                .iter()
                .map(|r| format!("- {r}"))
                .collect::<Vec<_>>()
                .join("\n"),
            self.observations
                .iter()
                .take(MAX_OBSERVATIONS)
                .map(|o| format!("- {o}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }

    pub fn format_short(&self) -> String {
        format!(
            "### {}\n\nCategory: {}\n\nnObservations:\n{}\n",
            self.name,
            self.category,
            self.observations
                .iter()
                .take(MAX_OBSERVATIONS)
                .map(|o| format!("- {o}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

impl Display for MemoryEntity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "### {}", self.name)?;
        writeln!(f, "\nType: {}", self.entity_type)?;
        writeln!(f, "\nImportance: {}", self.importance)?;
        writeln!(f, "\nRelations:")?;
        for relation in &self.relations {
            writeln!(f, "- {relation}")?;
        }
        writeln!(f, "\nObservations:")?;
        for observation in self.observations.iter().take(MAX_OBSERVATIONS) {
            writeln!(f, "- {observation}")?;
        }
        Ok(())
    }
}

impl MemoryExtractor {
    pub fn new(config: &Config) -> Result<Self> {
        Ok(Self {
            client: ClientBuilder::new()
                .default_headers({
                    let mut headers = reqwest::header::HeaderMap::new();
                    headers.insert(
                        "Authorization",
                        format!(
                            "Bearer {}",
                            config.provider_api_key.as_ref().unwrap().expose_secret()
                        )
                        .parse()?,
                    );
                    headers.insert("Content-Type", "application/json".parse().unwrap());
                    headers.insert("HTTP-Referer", "https://huly.io".parse().unwrap());
                    headers.insert("X-Title", "Huly".parse().unwrap());
                    headers
                })
                .build()?,
            system_prompt: include_str!("system_prompt.md")
                .replace("${NAME}", &config.huly.person.name),
            model: config.memory.extract_model.clone(),
        })
    }

    async fn extract(&self, context: &str, text: &str) -> Result<Vec<ExtractedMemoryEntity>> {
        let request = json!({
            "messages": [
                {
                    "role": "system",
                    "content": self.system_prompt,
                },
                {
                    "role": "user",
                    "content": context,
                },
                {
                    "role": "user",
                    "content": text,
                }
            ],
            "model": self.model,
            "temperature": 0.0,
        });
        let response = self
            .client
            .post(OPENROUTER_URL)
            .json(&request)
            .send()
            .await?
            .text()
            .await?;

        let response = serde_json::from_str::<serde_json::Value>(&response)
            .with_context(|| format!("Failed to parse response: {response}"))?;
        let Some(choices) = response
            .get("choices")
            .and_then(|choices| choices.as_array())
        else {
            tracing::warn!(%response, "No choices in response");
            return Ok(vec![]);
        };

        let Some(message) = choices
            .first()
            .and_then(|choice| choice.as_object().and_then(|choice| choice.get("message")))
        else {
            tracing::warn!(%response, "No message in response");
            return Ok(vec![]);
        };
        let Some(content) = message.get("content").and_then(|content| content.as_str()) else {
            tracing::warn!(%response, "No content in message");
            return Ok(vec![]);
        };

        if content.trim().starts_with("```json") {
            let content = content
                .trim_start_matches("```json")
                .trim_end_matches("```");
            let observations = serde_json::from_str::<Vec<ExtractedMemoryEntity>>(content)
                .with_context(|| format!("Failed to parse content: {content}"))?;
            tracing::info!(?observations, "Extracted memory entries");
            return Ok(observations);
        }
        tracing::warn!(%response, "No json formated content in message");
        Ok(vec![])
    }
}

async fn process_follow_chat(
    memory_extractor: &MemoryExtractor,
    db_client: &crate::database::DbClient,
    user_name: &str,
    task_id: i64,
    content: &str,
) -> Result<()> {
    // add initial channel log
    let mut result_message = format!("## Channel log \n{content}");
    let mut attempt_completion_message = "## Attempt completion \n".to_string();

    let messages = db_client.task_messages(task_id).await?;
    for message in messages.into_iter().skip(1) {
        match message {
            crate::types::Message::User { .. } => {}
            crate::types::Message::Assistant { content } => {
                let content = content.first().unwrap();
                match content {
                    crate::types::AssistantContent::Text(text) => {
                        if text.text.starts_with("<attempt_completion>") {
                            attempt_completion_message.push_str(
                                text.text
                                    .trim_start_matches("<attempt_completion>")
                                    .trim_end_matches("</attempt_completion>"),
                            );
                        }
                    }
                    crate::types::AssistantContent::ToolCall(tool_call) => {
                        if tool_call.function.name == "send_message" {
                            let content = tool_call
                                .function
                                .arguments
                                .get("content")
                                .and_then(|content| content.as_str())
                                .unwrap_or_default();

                            result_message.push_str(&format!(
                                "\n0|[_]({user_name}) _{}_:\n{content}\n",
                                Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                            ));
                        }
                    }
                }
            }
        }
    }

    let context = r#"
            <context>
            # Last Active Memory Entries
            ${ACTIVE_MEMORY_ENTRIES}

            # Relevant Memory Entries
            ${RELEVANT_MEMORY_ENTRIES}
            </context>
        "#;

    let text = format!("{result_message}\n{attempt_completion_message}");
    let active_memory_entries = db_client
        .mem_last_entities(MAX_MEMORY_ENTITIES)
        .await?
        .iter()
        .map(|e| e.format_short())
        .collect::<Vec<_>>()
        .join("\n");
    let relevant_memory_entries = db_client
        .mem_relevant_entities(MAX_MEMORY_ENTITIES, &text, MemoryEntityType::Semantic)
        .await?
        .iter()
        .map(|e| e.format_short())
        .collect::<Vec<_>>()
        .join("\n");

    let context = subst::substitute(
        context,
        &HashMap::from([
            ("ACTIVE_MEMORY_ENTRIES", &active_memory_entries),
            ("RELEVANT_MEMORY_ENTRIES", &relevant_memory_entries),
        ]),
    )?;
    let entities = memory_extractor.extract(&context, &text).await?;
    let importance_calculator = ImportanceCalculator::new();

    for mut ex_entity in entities {
        let entity = db_client
            .mem_entity_by_name(&ex_entity.entity_name, MemoryEntityType::Episode)
            .await;
        if let Some(mut entity) = entity {
            let mut observations = Vec::new();
            observations.append(&mut ex_entity.observations);
            // add old observations with filter existing
            observations.append(
                &mut entity
                    .observations
                    .drain(..)
                    .filter(|item| !observations.contains(item))
                    .collect(),
            );
            entity.observations = observations;
            entity.importance = 1.0;
            entity.access_count = entity.access_count.saturating_add(1);

            entity.updated_at = Utc::now();
            entity.importance = importance_calculator.calculate_importance(&entity);
            db_client.mem_update_entity(&entity).await?;
        } else {
            let mut entity = MemoryEntity {
                id: 0,
                name: ex_entity.entity_name,
                category: ex_entity.category,
                entity_type: MemoryEntityType::Episode,
                importance: 1.0,
                access_count: 0,
                observations: ex_entity.observations,
                relations: vec![],
                created_at: Default::default(),
                updated_at: Default::default(),
            };
            entity.importance = importance_calculator.calculate_importance(&entity);
            db_client.mem_add_entity(&entity).await?;
        }
    }
    Ok(())
}

async fn memory_mantainance(db_client: &crate::database::DbClient) -> Result<()> {
    let ids = db_client.mem_get_entity_ids().await?;
    let total_count = ids.len();
    tracing::info!("Memory entities count: {total_count}");
    let importance_calculator = ImportanceCalculator::new();
    let mut to_delete = vec![];
    for id in ids {
        let entity = db_client.mem_entity(id).await?;
        let importance = importance_calculator.calculate_importance(&entity);
        if importance < DELETE_THRESHOLD {
            to_delete.push(id);
        }
        db_client
            .mem_update_entity_importance(entity.id, importance)
            .await?;
    }
    let delete_count = to_delete.len();
    for id in to_delete {
        db_client.mem_delete_entity(id).await?;
    }
    tracing::info!(
        "Memory mantainance finished, processed: {total_count}, deleted: {delete_count}",
    );
    Ok(())
}

pub fn memory_worker(
    config: &Config,
    mut rx: UnboundedReceiver<Task>,
    db_client: crate::database::DbClient,
) -> Result<JoinHandle<()>> {
    let memory_extractor = MemoryExtractor::new(config)?;
    let user_name = config.huly.person.name.clone();
    let handler = tokio::spawn(async move {
        tracing::info!("Memory worker started");
        while let Some(task) = rx.recv().await {
            match task.kind {
                TaskKind::FollowChat { content, .. } => {
                    if let Err(e) = process_follow_chat(
                        &memory_extractor,
                        &db_client,
                        &user_name,
                        task.id,
                        &content,
                    )
                    .await
                    {
                        tracing::error!(?e, "Error processing task");
                    }
                }
                TaskKind::MemoryMantainance => {
                    if let Err(e) = memory_mantainance(&db_client).await {
                        tracing::error!(?e, "Error processing memory maintenance task");
                    }
                }
                _ => {}
            }
        }
        tracing::info!("Memory worker terminated");
    });
    Ok(handler)
}
