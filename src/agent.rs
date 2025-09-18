// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{collections::HashMap, time::Duration};

use anyhow::Result;
use serde_json::json;
use tokio::sync::mpsc;
use tracing::Level;
use wildcard::Wildcard;

use crate::{
    config::{self, Config, McpTransportConfig},
    context::AgentContext,
    providers::create_provider_client,
    state::AgentState,
    task::{Task, TaskFinishReason, TaskKind, TaskState},
    tools::{
        ToolImpl, ToolSet, browser::BrowserToolSet, command::CommandsToolSet, files::FilesToolSet,
        huly::create_huly_tool_set, notes::NotesToolSet, task::TaskToolSet, web::WebToolSet,
    },
};

const MAX_MEMORY_ENTITIES: u16 = 10;

mod assistant_chat_task;
mod channel_task;
mod sleep_task;
mod utils;

pub struct Agent {
    pub config: Config,
}

impl Agent {
    pub fn new(config: Config) -> Result<Self> {
        let this = Self { config };
        Ok(this)
    }

    async fn init_tools(
        config: &Config,
        context: &AgentContext,
        state: &AgentState,
    ) -> Result<(HashMap<String, Box<dyn ToolImpl>>, String, String)> {
        let mut tools: HashMap<String, Box<dyn ToolImpl>> = HashMap::default();
        let mut system_prompts = String::new();
        let mut tool_context = String::new();

        macro_rules! add_tool_set {
            ($tool_set:expr) => {
                let tool_set = $tool_set;
                let tool_set_name = tool_set.get_name();
                let new_tools = tool_set.get_tools(config, context, state).await;
                // tools sanity check
                for tool in new_tools.iter() {
                    if !tool.name().starts_with(tool_set_name) {
                        anyhow::bail!(
                            "Tool name '{}' must start with tool set name '{}'",
                            tool.name(),
                            tool_set_name
                        );
                    }
                }

                tools.extend(
                    new_tools
                        .into_iter()
                        .map(|t| (t.name().to_string(), t))
                        .collect::<HashMap<_, _>>(),
                );
                system_prompts.push_str(&tool_set.get_system_prompt(&config));
                tool_context.push_str(&tool_set.get_static_context(&config).await);
            };
        }

        add_tool_set!(create_huly_tool_set(config, context).await?);
        add_tool_set!(WebToolSet);
        add_tool_set!(TaskToolSet);
        add_tool_set!(NotesToolSet);
        add_tool_set!(FilesToolSet);
        add_tool_set!(CommandsToolSet);
        if let Some(browser) = &config.browser {
            let browser_toolset = BrowserToolSet::new(browser).await;
            add_tool_set!(browser_toolset);
        }

        // mcp tools
        #[cfg(feature = "mcp")]
        if let Some(mcp) = &config.mcp {
            use crate::tools::mcp::McpTool;
            use mcp_core::transport::ClientSseTransportBuilder;
            use mcp_core::types::ProtocolVersion;
            use serde_json::json;

            for (name, config) in mcp {
                use std::sync::Arc;

                use mcp_core::client::ClientBuilder;

                tracing::info!("Adding mcp tool {}", name);
                let McpTransportConfig::Sse { url, version } = &config.transport;
                let transport = ClientSseTransportBuilder::new(url.clone()).build();
                let client = ClientBuilder::new(transport)
                    .set_protocol_version(if version == ProtocolVersion::V2025_03_26.as_str() {
                        ProtocolVersion::V2025_03_26
                    } else {
                        ProtocolVersion::V2024_11_05
                    })
                    .build();

                client.open().await?;
                client.initialize().await?;
                let mcp_tools = client.list_tools(None, None).await?;
                let client_ref = Arc::new(client);
                mcp_tools.tools.into_iter().for_each(|tool| {
                    tools.insert(
                        tool.name.clone(),
                        Box::new(McpTool::new(
                            client_ref.clone(),
                            json!({
                                "function": {
                                    "description": tool.description,
                                    "name": tool.name,
                                    "parameters": tool.input_schema
                                },
                                "type": "function"
                            }),
                        )),
                    );
                });
            }
        }
        Ok((tools, system_prompts, tool_context))
    }

    pub async fn run(
        &self,
        task_receiver: mpsc::UnboundedReceiver<Task>,
        memory_task_sender: mpsc::UnboundedSender<Task>,
        mut context: AgentContext,
    ) -> Result<()> {
        tracing::info!("Start");

        let mut state = AgentState::new(context.db_client.clone()).await?;

        let (mut tools, tools_system_prompt, tools_context) =
            Self::init_tools(&self.config, &context, &state).await?;
        context.tools_context = Some(tools_context);
        context.tools_system_prompt = Some(tools_system_prompt);

        let tools_descriptions: HashMap<config::TaskKind, Vec<serde_json::Value>> = self
            .config
            .tasks
            .iter()
            .map(|(kind, cfg)| {
                (
                    kind.clone(),
                    cfg.tools
                        .iter()
                        .flat_map(|tool_pattern| {
                            let tool_pattern = Wildcard::new(tool_pattern.as_bytes()).unwrap();
                            tools.iter().filter_map(move |(key, tool)| {
                                if tool_pattern.is_match(key.as_bytes())
                                    && !tool.desciption().is_null()
                                {
                                    Some(tool.desciption().clone())
                                } else {
                                    None
                                }
                            })
                        })
                        .collect(),
                )
            })
            .collect();

        let provider_client = create_provider_client(&self.config)?;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Task>();
        let incoming_tasks_processor = utils::incoming_tasks_processor(
            task_receiver,
            memory_task_sender.clone(),
            context.db_client.clone(),
            tx,
        );

        // main agent loop
        loop {
            if let Some(mut task) = rx.recv().await {
                let span = tracing::span!(
                    Level::DEBUG,
                    "agent_task",
                    task_id = task.id,
                    task_kind = %task.kind
                );
                span.in_scope(async || -> Result<()> {
                    if let Some(channel_log_writer) = &context.channel_log_writer {
                        channel_log_writer
                            .trace_log(&format!("start task: {}, {}", task.id, task.kind));
                    }
                    tracing::info!("start task: {}, {}", task.id, task.kind);

                    let finish_reason = match task.kind {
                        TaskKind::Sleep => {
                            sleep_task::process_sleep_task(
                                &self.config,
                                provider_client.as_ref(),
                                &task,
                                &mut state,
                                &context,
                            )
                            .await
                        }
                        TaskKind::AssistantChat { .. } => {
                            assistant_chat_task::process_assistant_chat_task(
                                &self.config,
                                provider_client.as_ref(),
                                &mut tools,
                                &mut task,
                                &mut state,
                                &context,
                                &tools_descriptions[&config::TaskKind::AssistantChat],
                            )
                            .await
                        }
                        _ => {
                            channel_task::process_channel_task(
                                &self.config,
                                provider_client.as_ref(),
                                &mut tools,
                                &mut task,
                                &mut state,
                                &context,
                                &tools_descriptions[&config::TaskKind::FollowChat],
                            )
                            .await
                        }
                    };
                    if let Some(channel_log_writer) = &context.channel_log_writer {
                        channel_log_writer
                            .trace_log(&format!("task finished: {}, {:?}", task.id, finish_reason));
                    }

                    match finish_reason {
                        Ok(finish_reason) => match finish_reason {
                            TaskFinishReason::Completed => {
                                tracing::info!("Task complete: {}", task.id);
                                state.set_task_state(task.id, TaskState::Completed).await?;
                                let _ = memory_task_sender.send(task);
                            }
                            TaskFinishReason::Skipped => {
                                tracing::info!("Task skipped: {}", task.id);
                                state.set_task_state(task.id, TaskState::Postponed).await?;
                                let _ = memory_task_sender.send(task);
                            }
                            TaskFinishReason::Cancelled => {
                                tracing::info!("Task cancelled: {}", task.id);
                                state.set_task_state(task.id, TaskState::Cancelled).await?;
                            }
                        },
                        Err(e) => {
                            tracing::error!(?e, "Error processing task");
                            state.set_task_state(task.id, TaskState::Cancelled).await?;
                        }
                    }
                    Ok(())
                })
                .await?;
            } else {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if incoming_tasks_processor.is_finished() {
                break;
            }
        }

        Ok(())
    }
}
