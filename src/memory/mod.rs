// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{collections::HashMap, fmt::Display};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::{Client, ClientBuilder};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{sync::mpsc::UnboundedReceiver, task::JoinHandle};

use crate::{
    config::Config,
    task::{Task, TaskKind},
};

const MAX_OBSERVATIONS: usize = 20;
const MAX_MEMORY_ENTITIES: u16 = 10;

const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
struct MemoryExtractor {
    client: Client,
    system_prompt: String,
    model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExtractedMemoryEntity {
    pub entity_name: String,
    pub entity_type: String,
    pub observations: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MemoryEntity {
    pub id: i64,
    pub name: String,
    pub entity_type: String,
    pub importance: f64,
    pub access_count: i64,
    pub observations: Vec<String>,
    pub relations: Vec<String>,
    #[allow(dead_code)]
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl MemoryEntity {
    pub fn format(&self) -> String {
        format!(
            "### {}\n\nType: {}\n\nImportance: {}\n\nRelations:\n{}\n\nObservations:\n{}\n",
            self.name,
            self.entity_type,
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
            "### {}\n\nType: {}\n\nnObservations:\n{}\n",
            self.name,
            self.entity_type,
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
            .with_context(|| format!("Failed to parse response: {}", response))?;
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
            let observations = serde_json::from_str::<Vec<ExtractedMemoryEntity>>(content)?;
            tracing::info!(?observations, "Extracted memory entries");
            return Ok(observations);
        }
        tracing::warn!(%response, "No json formated content in message");
        Ok(vec![])
    }
}

async fn process_task(
    memory_extractor: &MemoryExtractor,
    db_client: &crate::database::DbClient,
    user_name: &str,
    task: Task,
) -> Result<()> {
    let TaskKind::FollowChat { content, .. } = task.kind else {
        return Ok(());
    };
    // add initial channel log
    let mut result_message = format!("## Channel log \n{content}");
    let mut attempt_completion_message = "## Attempt completion \n".to_string();

    let messages = db_client.task_messages(task.id).await?;
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
        .mem_relevant_entities(MAX_MEMORY_ENTITIES, &text)
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
    for mut ex_entity in entities {
        let entity = db_client.mem_entity_by_name(&ex_entity.entity_name).await;
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
            db_client.mem_update_entity(&entity).await?;
        } else {
            db_client
                .mem_add_entity(&MemoryEntity {
                    id: 0,
                    name: ex_entity.entity_name,
                    entity_type: ex_entity.entity_type,
                    importance: 1.0,
                    access_count: 0,
                    observations: ex_entity.observations,
                    relations: vec![],
                    created_at: Default::default(),
                    updated_at: Default::default(),
                })
                .await?;
        }
        // db_client.mem_add_entities(&mut [entry.clone()]).await?;
    }
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
            if let Err(e) = process_task(&memory_extractor, &db_client, &user_name, task).await {
                tracing::error!(?e, "Error processing task");
                continue;
            }
        }
        tracing::info!("Memory worker terminated");
    });
    Ok(handler)
}
