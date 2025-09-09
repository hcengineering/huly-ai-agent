// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{collections::HashMap, path::Path};

use base64::Engine;
use itertools::Itertools;
use tokio::{fs, sync::mpsc, task::JoinHandle};

use crate::{
    agent::{MAX_MEMORY_ENTITIES, utils::utils::normalize_path},
    config::Config,
    context::AgentContext,
    database::DbClient,
    memory::MemoryEntityType,
    state::AgentState,
    task::{MAX_FOLLOW_MESSAGES, Task, TaskKind},
    templates::{CONTEXT, SYSTEM_PROMPT},
    types::{
        AssistantContent, ContentFormat, Image, ImageMediaType, Message, Text, ToolCall,
        ToolResultContent, UserContent,
    },
    utils,
};

const MAX_FILES: usize = 1000;

pub async fn prepare_system_prompt(
    config: &Config,
    task_system_prompt: &str,
    tools_system_prompt: &str,
) -> String {
    let workspace_dir = config
        .workspace
        .as_os_str()
        .to_str()
        .unwrap()
        .replace("\\", "/");
    let person = &config.huly.person;
    let personality = format!(
        "- full name: {}\n- age: {}\n- sex: {}\n\n{}",
        person.name, person.age, person.sex, person.personality
    );
    let max_follow_messages = MAX_FOLLOW_MESSAGES.to_string();
    subst::substitute(
        SYSTEM_PROMPT,
        &HashMap::from([
            ("WORKSPACE_DIR", workspace_dir.as_str()),
            ("PERSONALITY", &personality),
            ("OS_NAME", std::env::consts::OS),
            (
                "OS_SHELL_EXECUTABLE",
                &std::env::var("SHELL").unwrap_or("sh".to_string()),
            ),
            ("USER_HOME_DIR", ""),
            ("TASK_SYSTEM_PROMPT", task_system_prompt),
            ("TOOLS_INSTRUCTION", tools_system_prompt),
            ("USER_INSTRUCTION", &config.user_instructions),
            ("MAX_FOLLOW_MESSAGES", &max_follow_messages),
        ]),
    )
    .unwrap()
}

pub async fn create_context(
    config: &Config,
    context: &AgentContext,
    state: &mut AgentState,
    messages: &[Message],
    task_context: &str,
) -> String {
    let workspace = config
        .workspace
        .as_os_str()
        .to_str()
        .unwrap()
        .replace("\\", "/");

    let mut result_context = CONTEXT.replace("${TASK_CONTEXT}", task_context);

    if result_context.contains("${BALANCE}") {
        result_context = result_context.replace("${BALANCE}", &state.balance().to_string());
    }

    if result_context.contains("${TIME}") {
        result_context = result_context.replace("${TIME}", &chrono::Utc::now().to_rfc2822());
    }

    if result_context.contains("${RGB_ROLES}") {
        let rgb_roles = format!(
            "# Three-Mind Discussion Protocol Roles\n- You - {}\n{}",
            config.huly.person.rgb_role,
            config
                .huly
                .person
                .rgb_opponents
                .iter()
                .map(|(person_id, role)| format!("- Person id {person_id} - {role}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        result_context = result_context.replace("${RGB_ROLES}", &rgb_roles);
    }

    if result_context.contains("${TOOLS_CONTEXT}") {
        result_context = result_context.replace(
            "${TOOLS_CONTEXT}",
            context.tools_context.as_ref().unwrap_or(&"".to_string()),
        );
    }

    if result_context.contains("${MEMORY_ENTRIES}") {
        let string_context = messages
            .iter()
            .map(|m| m.string_context())
            .collect::<Vec<_>>()
            .join("\n");
        let last_used_entities = context
            .db_client
            .mem_last_entities(MAX_MEMORY_ENTITIES)
            .await
            .unwrap();
        let relevant_entities = context
            .db_client
            .mem_relevant_entities(
                MAX_MEMORY_ENTITIES,
                &string_context,
                MemoryEntityType::Semantic,
            )
            .await
            .unwrap();

        result_context = result_context.replace(
            "${MEMORY_ENTRIES}",
            &format!(
                "\n# Last Active Memory Entries\n{}\n\n# Relevant Memory Entries\n{}\n",
                &last_used_entities
                    .iter()
                    .map(|e| e.format())
                    .collect::<Vec<_>>()
                    .join("\n"),
                &relevant_entities
                    .iter()
                    .map(|e| e.format())
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        );
    }

    if result_context.contains("${COMMANDS}") {
        let commands = context
            .process_registry
            .read()
            .await
            .processes()
            .map(|(id, status, command)| {
                format!(
                    "| {id}    | {}                 | `{command}` |",
                    if let Some(exit_status) = status {
                        format!("Exited({exit_status})")
                    } else {
                        "Running".to_string()
                    }
                )
            })
            .join("\n");
        result_context = result_context.replace(
            "${COMMANDS}",
            &format!("# Active Commands\n| Command ID | Status (Running/Exited) | Command |\n|------------|-------------------------|---------|\n{commands}\n"),
        );
    }

    if result_context.contains("${FILES}") {
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
        result_context = result_context.replace(
            "${FILES}",
            &format!("# Current Working Directory ({workspace}) Files (max depth 2)\n{files}"),
        );
    }
    result_context
}

pub fn has_send_message(messages: &[Message]) -> bool {
    messages.iter().any(|m| matches!(m, Message::Assistant{ content }
        if content.iter().any(|c|
            matches!(c, AssistantContent::ToolCall(ToolCall { function, .. }) if function.name == "send_message"))))
}

/// check messages for integrity and remove tool call messages without toolresult pair
pub fn check_integrity(messages: &mut Vec<Message>) -> bool {
    let mut ids_to_remove = vec![];
    for i in 0..messages.len() {
        let message = &messages[i];
        if let Message::Assistant { content } = message {
            if let Some(AssistantContent::ToolCall(tool_call)) = content.first() {
                let id = tool_call.id.clone();
                if let Some(Message::User { content }) = messages.get(i + 1) {
                    if let Some(UserContent::ToolResult(tool_result)) = content.first() {
                        if tool_result.id == id {
                            continue;
                        }
                    }
                }
                ids_to_remove.push(id);
            }
        }
    }

    messages.retain(|m| {
        if let Message::Assistant { content } = m {
            content.first().map(|c| match c {
                AssistantContent::ToolCall(tool_call) => !ids_to_remove.contains(&tool_call.id),
                _ => true,
            }) == Some(true)
        } else {
            true
        }
    });
    !ids_to_remove.is_empty()
}

async fn convert_image_content(workspace: &Path, image: &Image) -> Option<Text> {
    if image
        .format
        .as_ref()
        .is_none_or(|f| f == &ContentFormat::Base64)
    {
        if let Ok(data) = base64::engine::general_purpose::STANDARD.decode(image.data.clone()) {
            let uuid = uuid::Uuid::new_v4();
            let file_name = format!(
                "{uuid}.{}",
                image
                    .media_type
                    .as_ref()
                    .unwrap_or(&ImageMediaType::PNG)
                    .to_file_ext()
            );
            let file_path = normalize_path(workspace, &format!("images/{file_name}"));
            if fs::create_dir_all(Path::new(&file_path).parent().unwrap())
                .await
                .is_ok()
                && fs::write(&file_path, data).await.is_ok()
            {
                return Some(Text {
                    text: format!("Image saved to {file_path}"),
                });
            }
        }
    }
    None
}

pub async fn migrate_image_content(workspace: &Path, messages: &mut [Message]) -> bool {
    let mut migrated = false;
    let messages_count = messages.len();
    for (idx, message) in messages.iter_mut().enumerate() {
        if let Message::User { content } = message
            && idx < messages_count - 1
        {
            for content in content.iter_mut() {
                match content {
                    UserContent::Image(image) => {
                        if let Some(text) = convert_image_content(workspace, image).await {
                            *content = UserContent::Text(text);
                            migrated = true;
                        }
                    }
                    UserContent::ToolResult(tool_result) => {
                        if !tool_result.content.is_empty()
                            && let ToolResultContent::Image(image) = &tool_result.content[0]
                        {
                            if let Some(text) = convert_image_content(workspace, image).await {
                                tool_result.content[0] = ToolResultContent::Text(text);
                                migrated = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    migrated
}

pub fn incoming_tasks_processor(
    mut task_receiver: mpsc::UnboundedReceiver<Task>,
    memory_task_sender: mpsc::UnboundedSender<Task>,
    mut db_client: DbClient,
    tx: mpsc::UnboundedSender<Task>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut prev_task = Task::new(TaskKind::MemoryMantainance);
        // initialy load unfinished tasks from db
        for task in db_client.unfinished_tasks().await {
            let _ = tx.send(task.clone());
            prev_task = task;
        }
        while let Some(mut task) = task_receiver.recv().await {
            match task.kind {
                // for some task kind  we need just route the task
                TaskKind::MemoryMantainance => {
                    let _ = memory_task_sender.send(task);
                }
                _ => {
                    let id = db_client.add_task(&task).await.unwrap();
                    task.id = id;
                    if prev_task.kind.can_skip(&task.kind) {
                        prev_task.cancel_token.cancel();
                    }
                    let _ = tx.send(task.clone());
                    prev_task = task;
                }
            }
        }
    })
}
