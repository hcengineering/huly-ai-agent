// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Result;
use futures::StreamExt;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{
    config::{Config, McpTransportConfig},
    context::AgentContext,
    providers::create_provider_client,
    state::AgentState,
    task::{Task, TaskKind},
    templates::{CONTEXT, SYSTEM_PROMPT, TOOL_CALL_ERROR},
    tools::{ToolImpl, ToolSet, files::FilesToolSet, huly::HulyToolSet, memory::MemoryToolSet},
    types::{AssistantContent, Message, ToolCall},
};

const MESSAGE_COST: u32 = 50;
const MAX_FILES: usize = 1000;

pub struct Agent {
    pub config: Config,
    pub data_dir: PathBuf,
}

pub async fn prepare_system_prompt(
    workspace_dir: &Path,
    user_instructions: &str,
    tools_system_prompt: &str,
) -> String {
    let workspace_dir = workspace_dir
        .as_os_str()
        .to_str()
        .unwrap()
        .replace("\\", "/");
    subst::substitute(
        SYSTEM_PROMPT,
        &HashMap::from([
            ("WORKSPACE_DIR", workspace_dir.as_str()),
            ("OS_NAME", std::env::consts::OS),
            (
                "OS_SHELL_EXECUTABLE",
                &std::env::var("SHELL").unwrap_or("sh".to_string()),
            ),
            ("USER_HOME_DIR", ""),
            ("TOOLS_INSTRUCTION", tools_system_prompt),
            ("USER_INSTRUCTION", user_instructions),
        ]),
    )
    .unwrap()
}

pub async fn create_context(
    workspace: &Path,
    state: &mut AgentState,
    messages: &[Message],
) -> String {
    let workspace = workspace.as_os_str().to_str().unwrap().replace("\\", "/");
    let balance = state.balance();
    let string_context = messages
        .iter()
        .map(|m| m.string_context())
        .collect::<Vec<_>>()
        .join("\n");
    let relevant_entities = state
        .mem_list_relevant_entities(&string_context)
        .await
        .unwrap();
    let mut files: Vec<String> = Vec::default();
    for entry in ignore::WalkBuilder::new(&workspace)
        .filter_entry(|e| e.file_name() != "node_modules")
        .max_depth(Some(2))
        .build()
        .filter_map(|e| e.ok())
        .take(MAX_FILES)
    {
        files.push(
            entry
                .path()
                .to_str()
                .unwrap()
                .replace("\\", "/")
                .strip_prefix(&workspace)
                .unwrap()
                .to_string(),
        );
    }
    let files_str = files.join("\n");
    let files = if files.is_empty() {
        "No files found."
    } else {
        &files_str
    };

    // let commands = process_registry
    //     .read()
    //     .await
    //     .processes()
    //     .map(|(id, status, command)| {
    //         format!(
    //             "| {}    | {}                 | `{}` |",
    //             id,
    //             if let Some(exit_status) = status {
    //                 format!("Exited({})", exit_status)
    //             } else {
    //                 "Running".to_string()
    //             },
    //             command
    //         )
    //     })
    //     .join("\n");

    subst::substitute(
        CONTEXT,
        &HashMap::from([
            ("TIME", chrono::Local::now().to_rfc2822().as_str()),
            ("BALANCE", &balance.to_string()),
            ("WORKING_DIR", &workspace),
            (
                "MEMORY_ENTRIES",
                &serde_json::to_string_pretty(&relevant_entities).unwrap(),
            ),
            ("COMMANDS", ""),
            ("FILES", files),
        ]),
    )
    .unwrap()
}

fn has_send_message(messages: &[Message]) -> bool {
    messages.iter().any(|m| matches!(m, Message::Assistant{ content }
        if content.iter().any(|c|
            matches!(c, AssistantContent::ToolCall(ToolCall { function, .. }) if function.name == "send_message"))))
}

impl Agent {
    pub fn new(data_dir: &str, config: Config) -> Result<Self> {
        let this = Self {
            config,
            data_dir: Path::new(data_dir).to_path_buf(),
        };
        Ok(this)
    }

    async fn init_tools(
        config: &Config,
        context: &AgentContext,
        state: &AgentState,
    ) -> Result<(HashMap<String, Box<dyn ToolImpl>>, Vec<Value>, String)> {
        let mut tools: HashMap<String, Box<dyn ToolImpl>> = HashMap::default();
        let mut tools_description = vec![];
        let mut system_prompts = String::new();

        // files tools
        tools.extend(
            FilesToolSet::get_tools(config, context, state)
                .into_iter()
                .map(|t| (t.name().to_string(), t))
                .collect::<HashMap<_, _>>(),
        );
        tools_description.extend(FilesToolSet::get_tool_descriptions());
        system_prompts.push_str(FilesToolSet::get_system_prompt());

        // memory tools
        tools.extend(
            MemoryToolSet::get_tools(config, context, state)
                .into_iter()
                .map(|t| (t.name().to_string(), t))
                .collect::<HashMap<_, _>>(),
        );
        tools_description.extend(MemoryToolSet::get_tool_descriptions());
        system_prompts.push_str(MemoryToolSet::get_system_prompt());

        // huly tools
        tools.extend(
            HulyToolSet::get_tools(config, context, state)
                .into_iter()
                .map(|t| (t.name().to_string(), t))
                .collect::<HashMap<_, _>>(),
        );
        tools_description.extend(HulyToolSet::get_tool_descriptions());
        system_prompts.push_str(HulyToolSet::get_system_prompt());

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
                        Box::new(McpTool::new(tool.name.clone(), client_ref.clone())),
                    );

                    tools_description.push(json!({
                        "function": {
                            "description": tool.description,
                            "name": tool.name,
                            "parameters": tool.input_schema
                        },
                        "type": "function"
                    }));
                });
            }
        }
        Ok((tools, tools_description, system_prompts))
    }

    pub async fn run(
        &self,
        mut task_receiver: mpsc::UnboundedReceiver<Task>,
        context: AgentContext,
    ) -> Result<()> {
        // let mut tools: HashMap<String, Box<dyn ToolImpl>> = HashMap::default();
        tracing::info!("Start");

        let mut state = AgentState::new(self.data_dir.clone(), &self.config).await?;

        let (mut tools, tools_description, tools_system_prompt) =
            Self::init_tools(&self.config, &context, &state).await?;
        let provider_client = create_provider_client(&self.config, tools_description)?;

        // main agent loop
        let system_prompt = prepare_system_prompt(
            &self.config.workspace,
            &self.config.user_instructions,
            &tools_system_prompt,
        )
        .await;

        loop {
            while let Ok(task) = task_receiver.try_recv() {
                state.add_task(task).await?;
            }
            if let Some(mut task) = state.latest_task().await? {
                tracing::info!(?task, "start task");
                tracing::info!(log_message = true, "start task: {}", task.kind);

                let mut finished = false;
                let mut messages = state.task_messages(task.id).await?;
                if messages.is_empty() {
                    let message = task.kind.clone().to_message();
                    // add start message for task
                    messages.push(state.add_task_message(&mut task, message).await?);
                }
                loop {
                    if matches!(messages.last().unwrap(), Message::Assistant { .. }) {
                        match task.kind {
                            TaskKind::Mention { .. } => {
                                if has_send_message(&messages) {
                                    tracing::info!("Task complete: {:?}", task.kind);
                                    state.set_task_done(task.id).await?;
                                    continue;
                                } else {
                                    messages.push(state
                                    .add_task_message(
                                        &mut task,
                                        Message::user(
                                            "You need to use `send_message` tool to complete this task and finish the task with <attempt_completion> tag",
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
                    let context =
                        create_context(&self.config.workspace, &mut state, &messages).await;
                    //println!("context: {}", context);
                    let mut resp = provider_client
                        .send_messages(&system_prompt, &context, &messages)
                        .await?;
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
                                                    &mut task,
                                                    Message::assistant(&result_content),
                                                )
                                                .await?,
                                        );
                                        balance = balance.saturating_sub(MESSAGE_COST);
                                        if result_content.contains("<attempt_completion>") {
                                            finished = true;
                                        }
                                        result_content.clear();
                                    }
                                    messages.push(
                                        state
                                            .add_task_message(
                                                &mut task,
                                                Message::tool_call(tool_call.clone()),
                                            )
                                            .await?,
                                    );
                                    let tool_result = if let Some(tool) =
                                        tools.get_mut(&tool_call.function.name)
                                    {
                                        match tool.call(tool_call.function.arguments).await {
                                            Ok(tool_result) => tool_result,
                                            Err(e) => subst::substitute(
                                                TOOL_CALL_ERROR,
                                                &HashMap::from([("ERROR", &e.to_string())]),
                                            )
                                            .unwrap(),
                                        }
                                    } else {
                                        format!("Unknown tool [{}]", tool_call.function.name)
                                    };
                                    messages.push(
                                        state
                                            .add_task_message(
                                                &mut task,
                                                Message::tool_result(&tool_call.id, &tool_result),
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
                                .add_task_message(&mut task, Message::assistant(&result_content))
                                .await?,
                        );
                        balance = balance.saturating_sub(MESSAGE_COST);
                        if result_content.contains("<attempt_completion>") {
                            finished = true;
                        }
                        result_content.clear();
                    }
                    state.set_balance(balance).await?;
                    if finished {
                        tracing::info!("Task complete: {:?}", task.kind);
                        state.set_task_done(task.id).await?;
                        break;
                    }
                }
            } else {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}
