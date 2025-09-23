// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{collections::HashMap, time::Duration};

use anyhow::Result;
use chrono::Utc;
use futures::StreamExt;

use crate::{
    agent::utils,
    config::Config,
    context::AgentContext,
    memory::{MemoryEntity, MemoryEntityType},
    providers::ProviderClient,
    state::AgentState,
    task::{Task, TaskFinishReason},
    types::{AssistantContent, Message},
};

const MEMORY_CONSOLIDATION_THRESHOLD: f32 = 0.5;
const MEMORY_CONSOLIDATION_PAGE_SIZE: u16 = 20;
// 7 days
const TASK_EXPIRE_PERIOD: Duration = Duration::from_secs(60 * 60 * 24 * 7);

pub async fn process_sleep_task(
    config: &Config,
    provider_client: &dyn ProviderClient,
    task: &Task,
    state: &mut AgentState,
    context: &AgentContext,
) -> Result<TaskFinishReason> {
    let system_prompt = utils::prepare_system_prompt(
        config,
        &context.account_info,
        &task.kind.system_prompt(config),
        "",
    )
    .await;

    let ids = context
        .db_client
        .mem_entities_ids_for_consolidation(MEMORY_CONSOLIDATION_THRESHOLD)
        .await?;
    let total_count = ids.len();
    let mut semantic_count = 0;
    for ids in ids.chunks(MEMORY_CONSOLIDATION_PAGE_SIZE.into()) {
        let count = ids.len();
        let mut semantic_entities: HashMap<String, MemoryEntity> = HashMap::new();
        let mut mem_entities = vec![];
        for id in ids {
            let mem_entity = context.db_client.mem_entity(*id).await?;
            let query = format!(
                "{}\n{}\n{}",
                mem_entity.name,
                mem_entity.category,
                mem_entity.observations.join("\n")
            );
            let relevant_sem_entities = context
                .db_client
                .mem_relevant_entities(3, &query, MemoryEntityType::Semantic)
                .await?;
            mem_entities.push(mem_entity);
            for entity in relevant_sem_entities.into_iter() {
                if !semantic_entities.contains_key(&entity.name) {
                    semantic_entities.insert(entity.name.clone(), entity);
                }
            }
        }
        tracing::info!("Processing {count} memory items");

        let messages = vec![Message::user(&serde_json::to_string_pretty(
            &mem_entities
                .iter()
                .chain(semantic_entities.values())
                .collect::<Vec<_>>(),
        )?)];
        let evn_context = utils::create_context(
            config,
            context,
            state,
            &messages,
            &task.kind.context(config, context),
        )
        .await;
        let mut resp = provider_client
            .send_messages(&system_prompt, &evn_context, &messages, &[])
            .await?;
        let mut result_content = String::new();
        while let Some(result) = resp.next().await {
            match result {
                Ok(content) => {
                    if let AssistantContent::Text(text) = content {
                        result_content.push_str(&text.text);
                    }
                }
                Err(e) => {
                    tracing::error!(?e, "Error processing message");
                }
            }
        }
        if result_content.starts_with("```json") {
            result_content = result_content
                .trim_start_matches("```json")
                .trim_end_matches("```")
                .to_string();
        }
        let new_entities: Vec<MemoryEntity> = serde_json::from_str(&result_content)?;
        semantic_count += new_entities.len();
        for mut entity in new_entities.into_iter() {
            if let Some(sem_entity) = semantic_entities.get(&entity.name) {
                // if semantic was in current request we should rewrite it
                tracing::debug!(
                    "rewrite existing semantic entity {}: {} ",
                    sem_entity.id,
                    sem_entity.name
                );
                entity.id = sem_entity.id;
                entity.importance = sem_entity.importance;
            } else if let Some(existing_entity) = context
                .db_client
                .mem_entity_by_name(&entity.name, MemoryEntityType::Semantic)
                .await
            {
                // check in db if entity already exists
                tracing::debug!(
                    "rewrite existing semantic entity {}: {} ",
                    existing_entity.id,
                    existing_entity.name
                );
                entity.id = existing_entity.id;
                entity.importance = existing_entity.importance;
            }

            // write to db
            for entity_id in ids {
                context.db_client.mem_delete_entity(*entity_id).await?;
            }
            entity.updated_at = Utc::now();
            if entity.id == 0 {
                entity.importance = 1.0;
                context.db_client.mem_add_entity(&entity).await?;
            } else {
                entity.importance = (entity.importance * 1.5).min(1.0);
                context.db_client.mem_update_entity(&entity).await?;
            }
        }
    }

    tracing::info!(
        "Memory process finished: stored {semantic_count} semantic entities, delete {total_count} episodic entities"
    );
    context
        .db_client
        .delete_old_tasks(Utc::now() - TASK_EXPIRE_PERIOD)
        .await?;
    Ok(TaskFinishReason::Completed)
}
