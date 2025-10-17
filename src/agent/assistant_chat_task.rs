// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::collections::HashMap;

use anyhow::Result;
use futures::StreamExt;
use regex::Regex;
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

pub async fn process_assistant_chat_task(
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

    let TaskKind::AssistantChat {
        ref card_id,
        ref message_id,
        ref content,
        ..
    } = task.kind
    else {
        return Ok(TaskFinishReason::Skipped);
    };

    let mut finished = false;
    let mut messages: Vec<Message> = state.get_assistant_messages(card_id).await?;

    // remove first non-user messages
    messages = messages
        .drain(..)
        .skip_while(|m| !m.is_user_message())
        .collect();

    messages.push(Message::user(&format!(
        "<message_id>{message_id}</message_id>{}",
        content
    )));

    let mut result_content = String::new();
    let mut mood = None;

    async fn check_result_content(
        context: &AgentContext,
        card_id: &str,
        result_content: &mut String,
        finished: &mut bool,
        messages: &mut Vec<Message>,
        mood: &mut Option<String>,
        trace_info: &str,
    ) {
        if !result_content.is_empty() {
            tracing::trace!(trace_info, result_content);
            if result_content.contains("<|done|>") {
                *finished = true;
                *result_content = result_content.replace("<|done|>", "").trim().to_string();
            }
            let regex = Regex::new(r"<\|([a-zA-Z\s]+)\|>").unwrap();
            if let Some(caps) = regex.captures(result_content) {
                let current_mood = caps[1].to_string();
                tracing::debug!("Mood: {current_mood}");
                *mood = Some(current_mood);
            }
            *result_content = regex.replace_all(result_content, "").to_string();
            if !result_content.is_empty() {
                messages.push(Message::assistant(result_content));
                huly::send_message(
                    &context.tx_client,
                    card_id,
                    &context.account_info.social_id,
                    result_content,
                )
                .await
                .ok();
            }
            context.typing_client.reset_typing(card_id).await.ok();
            result_content.clear();
        }
    }

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
        if let Err(err) = context
            .typing_client
            .set_typing(card_id, Some("Thinking".to_string()), 5)
            .await
        {
            tracing::warn!(?err, "Failed to set typing");
        }

        let send_messages = provider_client.send_messages(
            &system_prompt,
            &evn_context,
            &messages,
            tools_descriptions,
        );
        let mut resp = select! {
            _ = task.cancel_token.cancelled() => {
                state.set_assistant_messages(card_id, &messages).await?;
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
                        check_result_content(
                            context,
                            card_id,
                            &mut result_content,
                            &mut finished,
                            &mut messages,
                            &mut mood,
                            "assistant response with tool call",
                        )
                        .await;
                        messages.push(Message::tool_call(tool_call.clone()));
                        let tool_result = if let Some(tool) =
                            tools.get_mut(&tool_call.function.name)
                        {
                            let span = tracing::span!(
                                Level::INFO,
                                "tool_call",
                                call_id = tool_call.id,
                                name = tool_call.function.name
                            );
                            context
                                .typing_client
                                .set_typing(
                                    card_id,
                                    Some(format!("Call {} tool", tool_call.function.name)),
                                    5,
                                )
                                .await
                                .ok();
                            match span.in_scope(async || -> std::result::Result<Vec<ToolResultContent>, TaskFinishReason> {
                                    let tool_call = tool.call(context, tool_call.function.arguments);
                                    Ok(select! {
                                        _ = task.cancel_token.cancelled() => {
                                            state.set_assistant_messages(card_id, &messages).await.ok();
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
                        messages.push(Message::tool_result(&tool_call.id, tool_result));
                    }
                },
                Err(e) => {
                    tracing::error!(?e, "Error processing message");
                }
            }
        }
        check_result_content(
            context,
            card_id,
            &mut result_content,
            &mut finished,
            &mut messages,
            &mut mood,
            "assistant response",
        )
        .await;
        state.set_assistant_messages(card_id, &messages).await?;

        if finished {
            break;
        }

        if last_message_count == messages.len() {
            tracing::warn!("Task produced no messages");
            return Ok(TaskFinishReason::Cancelled);
        }
    }

    if let Some(mood) = mood {
        context
            .typing_client
            .set_typing(card_id, Some(mood), 5)
            .await
            .ok();
    }

    Ok(if finished {
        TaskFinishReason::Completed
    } else {
        TaskFinishReason::Skipped
    })
}
