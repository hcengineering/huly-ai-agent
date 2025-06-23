use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::RwLock,
    task::Poll,
    time::Duration,
};

use anyhow::Result;
use futures::StreamExt;
use serde_json::Value;
use tokio::{pin, sync::mpsc};

use crate::{
    config::Config,
    providers::create_provider_client,
    state::AgentState,
    task::Task,
    templates::SYSTEM_PROMPT,
    tools::{files::FilesToolSet, ToolImpl, ToolSet},
    types::{AssistantContent, Message},
};

const MESSAGE_COST: u32 = 50;

pub struct Agent {
    pub config: Config,
    pub data_dir: PathBuf,
}

fn store_state(state: &AgentState, data_dir: &Path) -> Result<()> {
    let state_path = data_dir.join("state.json");
    let state_str = serde_json::to_string(&state)?;
    std::fs::write(state_path, state_str)?;
    Ok(())
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

// pub async fn add_env_message<'a>(
//     msg: &'a mut Message,
//     memory_index: Arc<
//         tokio::sync::RwLock<InMemoryVectorIndex<rig_fastembed::EmbeddingModel, memory::Entity>>,
//     >,
//     workspace: &'a Path,
//     process_registry: Arc<RwLock<ProcessRegistry>>,
// ) {
//     let workspace = workspace.as_os_str().to_str().unwrap().replace("\\", "/");
//     let mut files: Vec<String> = Vec::default();

//     for entry in ignore::WalkBuilder::new(&workspace)
//         .filter_entry(|e| e.file_name() != "node_modules")
//         .max_depth(Some(2))
//         .build()
//         .filter_map(|e| e.ok())
//         .take(MAX_FILES)
//     {
//         files.push(
//             entry
//                 .path()
//                 .to_str()
//                 .unwrap()
//                 .replace("\\", "/")
//                 .strip_prefix(&workspace)
//                 .unwrap()
//                 .to_string(),
//         );
//     }
//     let files_str = files.join("\n");
//     let files = if files.is_empty() {
//         "No files found."
//     } else {
//         &files_str
//     };
//     if let Message::User { content } = msg {
//         let text = content.first();
//         let mut memory_entries = String::new();
//         let memory_index = memory_index.read().await;
//         let txt = match text {
//             UserContent::Text(text) => &text.text.to_string(),
//             UserContent::ToolResult(tool_result) => match tool_result.content.first() {
//                 rig::message::ToolResultContent::Text(text) => &text.text.to_string(),
//                 rig::message::ToolResultContent::Image(_) => "",
//             },
//             _ => "",
//         };
//         if !txt.is_empty() {
//             let res: Vec<(f64, String, Entity)> = memory_index.top_n(txt, 10).await.unwrap();
//             let result: Vec<_> = res.into_iter().map(|(_, _, entity)| entity).collect();
//             memory_entries = serde_yaml::to_string(&result).unwrap();
//         }

//         let commands = process_registry
//             .read()
//             .await
//             .processes()
//             .map(|(id, status, command)| {
//                 format!(
//                     "| {}    | {}                 | `{}` |",
//                     id,
//                     if let Some(exit_status) = status {
//                         format!("Exited({})", exit_status)
//                     } else {
//                         "Running".to_string()
//                     },
//                     command
//                 )
//             })
//             .join("\n");
//         content.push(UserContent::text(
//             subst::substitute(
//                 ENV_DETAILS,
//                 &HashMap::from([
//                     ("TIME", chrono::Local::now().to_rfc2822().as_str()),
//                     ("WORKING_DIR", &workspace),
//                     ("MEMORY_ENTRIES", &memory_entries),
//                     ("COMMANDS", &commands),
//                     ("FILES", files),
//                 ]),
//             )
//             .unwrap(),
//         ));
//     }
// }

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
    ) -> Result<(HashMap<String, Box<dyn ToolImpl>>, Vec<Value>, String)> {
        let mut tools: HashMap<String, Box<dyn ToolImpl>> = HashMap::default();
        let mut tools_description = vec![];
        let mut system_prompts = String::new();

        // files tools
        tools.extend(
            FilesToolSet::get_tools(&config)
                .into_iter()
                .map(|t| (t.name().to_string(), t))
                .collect::<HashMap<_, _>>(),
        );
        tools_description.extend(FilesToolSet::get_tool_descriptions());
        system_prompts.push_str(FilesToolSet::get_system_prompt());
        Ok((tools, tools_description, system_prompts))
    }

    pub async fn run(&self, mut task_receiver: mpsc::UnboundedReceiver<Task>) -> Result<()> {
        // let mut tools: HashMap<String, Box<dyn ToolImpl>> = HashMap::default();
        tracing::info!("Start");

        let mut state = AgentState {
            tasks: vec![],
            balance: 1000,
        };

        let (tools, tools_description, tools_system_prompt) =
            Self::init_tools(&self.config).await?;
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
                state.tasks.push(task);
            }
            if let Some(mut task) = state.tasks.pop() {
                tracing::info!("Task: {:?}", task);
                for _ in 0..3 {
                    let mut resp = provider_client
                        .send_messages(&system_prompt, "some context", &task.messages)
                        .await?;
                    let mut result_content = String::new();
                    let mut balance = state.balance;
                    while let Some(result) = resp.next().await {
                        match result {
                            Ok(content) => match content {
                                AssistantContent::Text(text) => {
                                    result_content.push_str(&text.text);
                                }
                                AssistantContent::ToolCall(tool_call) => {
                                    tracing::info!("Tool call: {:?}", tool_call);
                                    if !result_content.is_empty() {
                                        task.messages.push(Message::assistant(&result_content));
                                        balance = balance.saturating_sub(MESSAGE_COST);
                                        result_content.clear();
                                    }
                                    task.messages.push(Message::tool_call(tool_call.clone()));
                                    let tool_result =
                                        if let Some(tool) = tools.get(&tool_call.function.name) {
                                            tool.call(tool_call.function.arguments).await?
                                        } else {
                                            format!("Unknown tool [{}]", tool_call.function.name)
                                        };
                                    task.messages.push(Message::assistant(&tool_result));
                                    balance = balance.saturating_sub(MESSAGE_COST);
                                }
                            },
                            Err(e) => {
                                println!("{:?}", e);
                            }
                        }
                    }
                    if !result_content.is_empty() {
                        task.messages.push(Message::assistant(&result_content));
                        balance = balance.saturating_sub(MESSAGE_COST);
                        result_content.clear();
                    }
                    state.balance = balance;
                    store_state(&state, &self.data_dir)?;
                    tracing::info!("Task complete: {:?}", task);
                }
            } else {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}
