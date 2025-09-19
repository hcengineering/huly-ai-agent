// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use anyhow::Result;
use futures::StreamExt;
use tokio::select;
use tracing::Level;

use crate::{
    agent::utils,
    config::Config,
    context::AgentContext,
    huly,
    providers::ProviderClient,
    state::AgentState,
    task::{Task, TaskFinishReason, TaskKind},
    templates::TOOL_CALL_ERROR,
    tools::ToolImpl,
    types::{AssistantContent, Message, ToolResultContent},
};

const MESSAGE_COST: u32 = 50;
const WAIT_REACTION_COMPLEXITY: u32 = 20;
const WAIT_REACTION_DURATION: Duration = Duration::from_secs(60);
const MAX_STEPS_PER_COMPLEXITY: usize = 2;

pub async fn process_channel_task(
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
        &task.kind.system_prompt(config),
        context.tools_system_prompt.as_ref().unwrap(),
    )
    .await;

    async fn add_reaction(
        context: &AgentContext,
        task_kind: &TaskKind,
        reaction: &str,
    ) -> Result<()> {
        if let TaskKind::FollowChat {
            channel_id,
            message_id,
            ..
        } = task_kind
        {
            huly::add_reaction(
                &context.tx_client,
                channel_id,
                message_id,
                &context.social_id,
                reaction,
            )
            .await?;
        }
        Ok(())
    }

    let mut finished = false;
    let mut messages = state.task_messages(task.id).await?;
    // remove last assistant message if it is Assistant
    let mut changed = utils::check_integrity(&mut messages);
    changed = changed || utils::migrate_image_content(&config.workspace, &mut messages).await;

    if changed {
        state.update_task_messages(task.id, &messages).await;
    }
    if messages.is_empty() {
        let message = task.kind.clone().to_message();
        // add start message for task
        messages.push(state.add_task_message(context, task, message).await?);
    }
    let start_time = Instant::now();
    let mut wait_reaction_added = false;
    loop {
        if matches!(messages.last().unwrap(), Message::Assistant { .. }) {
            match task.kind {
                TaskKind::FollowChat { .. } => {
                    if utils::has_send_message(&messages) {
                        tracing::info!("Task complete: {:?}", task.kind);
                        state.set_task_done(task.id).await?;
                        continue;
                    } else {
                        messages.push(state
                                    .add_task_message(context,
                                        task,
                                        Message::user(
                                            "You need to use `send_message` tool to complete this task or finish the task with <attempt_completion> tag",
                                        ),
                                    )
                                    .await?);
                    }
                }
                _ => {
                    //TODO: handle other tasks
                }
            }
        }
        let evn_context = utils::create_context(
            config,
            context,
            state,
            &messages,
            &task.kind.context(config),
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

        let mut result_content = String::new();
        let mut balance = state.balance();
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
                                    .add_task_message(
                                        context,
                                        task,
                                        Message::assistant(&result_content),
                                    )
                                    .await?,
                            );
                            balance = balance.saturating_sub(MESSAGE_COST);
                            if let Some(complexity) =
                                state.update_task_complexity(task, &result_content).await
                            {
                                if complexity > WAIT_REACTION_COMPLEXITY && !wait_reaction_added {
                                    add_reaction(context, &task.kind, "👀").await?;
                                    wait_reaction_added = true;
                                }
                            }
                            if result_content.contains("<attempt_completion>") {
                                finished = true;
                            }
                            result_content.clear();
                        }
                        messages.push(
                            state
                                .add_task_message(
                                    context,
                                    task,
                                    Message::tool_call(tool_call.clone()),
                                )
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
                                    context,
                                    task,
                                    Message::tool_result(&tool_call.id, tool_result),
                                )
                                .await?,
                        );
                        balance = balance.saturating_sub(MESSAGE_COST);
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
                    .add_task_message(context, task, Message::assistant(&result_content))
                    .await?,
            );
            balance = balance.saturating_sub(MESSAGE_COST);
            if let Some(complexity) = state.update_task_complexity(task, &result_content).await {
                if complexity > WAIT_REACTION_COMPLEXITY && !wait_reaction_added {
                    add_reaction(context, &task.kind, "👀").await?;
                    wait_reaction_added = true;
                }
            }
            if result_content.contains("<attempt_completion>") {
                finished = true;
            }
            result_content.clear();
        }

        if utils::migrate_image_content(&config.workspace, &mut messages).await {
            state.update_task_messages(task.id, &messages).await;
        }
        state.set_balance(balance).await?;
        if finished {
            break;
        }
        if messages.len() > MAX_STEPS_PER_COMPLEXITY * task.complexity as usize {
            tracing::info!("Task steps limit reached");
            add_reaction(context, &task.kind, "❌").await?;
            return Ok(TaskFinishReason::Cancelled);
        }
        if !wait_reaction_added
            && Instant::now().saturating_duration_since(start_time) > WAIT_REACTION_DURATION
        {
            add_reaction(context, &task.kind, "👀").await?;
            wait_reaction_added = true
        }
    }

    Ok(if finished {
        TaskFinishReason::Completed
    } else {
        TaskFinishReason::Skipped
    })
}
