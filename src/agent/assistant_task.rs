// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::collections::HashMap;

use anyhow::Result;
use futures::StreamExt;
use tokio::select;
use tracing::Level;

use crate::{
    agent::utils,
    config::Config,
    context::AgentContext,
    providers::ProviderClient,
    state::AgentState,
    task::{Task, TaskFinishReason, TaskKind},
    templates::TOOL_CALL_ERROR,
    tools::ToolImpl,
    types::{AssistantContent, Message, ToolResultContent},
};

pub async fn process_assistant_task(
    config: &Config,
    provider_client: &dyn ProviderClient,
    tools: &mut HashMap<String, Box<dyn ToolImpl>>,
    task: &mut Task,
    state: &mut AgentState,
    context: &AgentContext,
    tools_descriptions: &[serde_json::Value],
) -> Result<TaskFinishReason> {
    let system_prompt = utils::prepare_system_prompt(
        config,
        &context.account_info,
        &task.kind.system_prompt(config),
        context.tools_system_prompt.as_ref().unwrap(),
    )
    .await;

    let TaskKind::AssistantTask { ref content, .. } = task.kind else {
        return Ok(TaskFinishReason::Skipped);
    };

    let mut finished = false;
    let mut messages = state.task_messages(task.id).await?;
    // remove last assistant message if it is Assistant
    let mut changed = utils::check_integrity(&mut messages);
    changed = changed || utils::migrate_image_content(&config.workspace, &mut messages).await;

    if changed {
        state.update_task_messages(task.id, &messages).await;
    }
    if messages.is_empty() {
        // add start message for task
        messages.push(state.add_task_message(task, Message::user(content)).await?);
    }

    let mut result_content = String::new();
    let mut last_message_count;

    loop {
        last_message_count = messages.len();
        let evn_context = utils::create_context(
            config,
            context,
            state,
            &messages,
            &task.kind.context(config, context),
        )
        .await;

        let send_messages = provider_client.send_messages(
            &system_prompt,
            &evn_context,
            &messages,
            tools_descriptions,
        );
        let mut resp = select! {
            _ = task.cancel_token.cancelled() => {
                return Ok(TaskFinishReason::Cancelled);
            },
            result = send_messages => {
                result?
            }
        };

        result_content.clear();

        while let Some(result) = resp.next().await {
            match result {
                Ok(content) => match content {
                    AssistantContent::Text(text) => {
                        result_content.push_str(&text.text);
                    }
                    AssistantContent::ToolCall(tool_call) => {
                        tracing::trace!(?tool_call, "Tool call");
                        if !result_content.is_empty() {
                            messages.push(
                                state
                                    .add_task_message(task, Message::assistant(&result_content))
                                    .await?,
                            );

                            if result_content.contains("<|done|>") {
                                finished = true;
                            }
                            result_content.clear();
                        }
                        messages.push(
                            state
                                .add_task_message(task, Message::tool_call(tool_call.clone()))
                                .await?,
                        );

                        let tool_result = if let Some(tool) =
                            tools.get_mut(&tool_call.function.name)
                        {
                            let span = tracing::span!(
                                Level::INFO,
                                "tool_call",
                                call_id = tool_call.id,
                                name = tool_call.function.name
                            );
                            match span.in_scope(async || -> std::result::Result<Vec<ToolResultContent>, TaskFinishReason> {
                                    let tool_call = tool.call(context, tool_call.function.arguments);
                                    Ok(select! {
                                        _ = task.cancel_token.cancelled() => {
                                            return std::result::Result::Err(TaskFinishReason::Cancelled);
                                        },
                                        res = tool_call => {
                                            match res {
                                                Ok(tool_result) => tool_result,
                                                Err(e) => vec![ToolResultContent::text(
                                                    subst::substitute(
                                                        TOOL_CALL_ERROR,
                                                        &HashMap::from([("ERROR", &e.to_string())]),
                                                    )
                                                    .unwrap(),
                                                )],
                                            }
                                        }
                                    })
                                })
                                .await {
                                    Ok(result) => result,
                                    Err(reason) => {
                                        return Ok(reason);
                                    }
                                }
                        } else {
                            vec![ToolResultContent::text(format!(
                                "Unknown tool [{}]",
                                tool_call.function.name
                            ))]
                        };
                        messages.push(
                            state
                                .add_task_message(
                                    task,
                                    Message::tool_result(&tool_call.id, tool_result),
                                )
                                .await?,
                        );
                    }
                },
                Err(e) => {
                    tracing::error!(?e, "Error processing message");
                }
            }
        }
        if !result_content.is_empty() {
            messages.push(
                state
                    .add_task_message(task, Message::assistant(&result_content))
                    .await?,
            );
            if result_content.contains("<|done|>") {
                finished = true;
            }
            result_content.clear();
        }

        if utils::migrate_image_content(&config.workspace, &mut messages).await {
            state.update_task_messages(task.id, &messages).await;
        }

        if finished {
            break;
        }

        if last_message_count == messages.len() {
            tracing::warn!("Task produced no messages");
            return Ok(TaskFinishReason::Cancelled);
        }
    }

    Ok(if finished {
        TaskFinishReason::Completed
    } else {
        TaskFinishReason::Skipped
    })
}
