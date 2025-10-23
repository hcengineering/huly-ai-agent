// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::collections::HashMap;

use anyhow::Result;
use futures::StreamExt;

use crate::{
    agent::utils,
    config::Config,
    context::AgentContext,
    providers::ProviderClient,
    state::AgentState,
    task::TaskKind,
    templates::TOOL_CALL_ERROR,
    tools::ToolImpl,
    types::{AssistantContent, Message, ToolResultContent},
};

pub async fn notes_mantainance(
    config: &Config,
    provider_client: &dyn ProviderClient,
    tools: &mut HashMap<String, Box<dyn ToolImpl>>,
    state: &mut AgentState,
    context: &AgentContext,
    tools_descriptions: &[serde_json::Value],
) -> Result<()> {
    let task_kind = TaskKind::NotesMantainance;
    let system_prompt = utils::prepare_system_prompt(
        config,
        &context.account_info,
        &task_kind.system_prompt(config),
        context.tools_system_prompt.as_ref().unwrap(),
    )
    .await;
    let mut messages = vec![task_kind.to_message()];
    let mut result_content = String::new();
    let mut last_message_count;
    let mut finished = false;

    loop {
        last_message_count = messages.len();
        let evn_context = utils::create_context(
            config,
            context,
            state,
            &messages,
            &task_kind.context(config, context),
        )
        .await;

        let mut resp = provider_client
            .send_messages(&system_prompt, &evn_context, &messages, tools_descriptions)
            .await?;

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
                            messages.push(Message::assistant(&result_content));

                            if result_content.contains("<|done|>") {
                                finished = true;
                            }
                            result_content.clear();
                        }
                        messages.push(Message::tool_call(tool_call.clone()));

                        let tool_result =
                            if let Some(tool) = tools.get_mut(&tool_call.function.name) {
                                let res = tool.call(context, tool_call.function.arguments).await;
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
        if !result_content.is_empty() {
            messages.push(Message::assistant(&result_content));
            if result_content.contains("<|done|>") {
                finished = true;
            }
            result_content.clear();
        }

        if finished {
            break;
        }

        if last_message_count == messages.len() {
            tracing::warn!("Task produced no messages");
            return Ok(());
        }
    }

    Ok(())
}
