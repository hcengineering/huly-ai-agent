// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    vec,
};

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::{
    config::Config,
    context::AgentContext,
    state::AgentState,
    tools::{ToolImpl, ToolSet, command::process_registry::ProcessRegistry},
    types::ToolResultContent,
};

pub mod process_registry;

const COMMAND_TIMEOUT: u64 = 300; // 30 secs

pub struct CommandsToolSet;

impl ToolSet for CommandsToolSet {
    fn get_name(&self) -> &str {
        "cmd"
    }

    async fn get_tools<'a>(
        &self,
        config: &'a Config,
        context: &'a AgentContext,
        _state: &'a AgentState,
    ) -> Vec<Box<dyn ToolImpl>> {
        let mut descriptions = serde_json::from_str::<Vec<serde_json::Value>>(
            &include_str!("tools.json")
                .replace("${WORKSPACE}", &workspace_to_string(&config.workspace)),
        )
        .unwrap()
        .into_iter()
        .map(|v| (v["function"]["name"].as_str().unwrap().to_string(), v))
        .collect::<HashMap<String, serde_json::Value>>();

        vec![
            Box::new(ExecuteCommandTool {
                workspace: config.workspace.clone(),
                process_registry: context.process_registry.clone(),
                description: descriptions.remove("cmd_exec").unwrap(),
            }),
            Box::new(GetCommandResultTool {
                process_registry: context.process_registry.clone(),
                description: descriptions.remove("cmd_get_result").unwrap(),
            }),
            Box::new(TerminateCommandTool {
                process_registry: context.process_registry.clone(),
                description: descriptions.remove("cmd_terminate").unwrap(),
            }),
        ]
    }

    fn get_system_prompt(&self, config: &Config) -> String {
        include_str!("system_prompt.txt")
            .replace("${WORKSPACE}", &workspace_to_string(&config.workspace))
    }
}

#[inline]
fn workspace_to_string(workspace: &Path) -> String {
    workspace.to_str().unwrap().to_string().replace("\\", "/")
}

struct ExecuteCommandTool {
    workspace: PathBuf,
    process_registry: Arc<RwLock<ProcessRegistry>>,
    description: serde_json::Value,
}

#[derive(Deserialize)]
struct ExecuteCommandToolArgs {
    command: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GetCommandResultToolArgs {
    pub command_id: usize,
}

pub struct GetCommandResultTool {
    process_registry: Arc<RwLock<ProcessRegistry>>,
    description: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TerminateCommandToolArgs {
    pub command_id: usize,
}

pub struct TerminateCommandTool {
    process_registry: Arc<RwLock<ProcessRegistry>>,
    description: serde_json::Value,
}

#[async_trait]
impl ToolImpl for ExecuteCommandTool {
    fn desciption(&self) -> &serde_json::Value {
        &self.description
    }

    async fn call(&mut self, args: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<ExecuteCommandToolArgs>(args)?;
        tracing::info!("Execute command '{}'", args.command);
        let command_id = self
            .process_registry
            .write()
            .await
            .execute_command(&args.command, &workspace_to_string(&self.workspace))
            .await?;
        let mut command_output = String::new();
        for _ in 0..COMMAND_TIMEOUT {
            self.process_registry.write().await.poll();
            if let Some((exit_status, output)) =
                self.process_registry.read().await.get_process(command_id)
            {
                if let Some(exit_status) = exit_status {
                    return Ok(vec![ToolResultContent::text(format!(
                        "Command ID: {command_id}\nExit Status: Exited({exit_status})\nOutput:\n{output}"
                    ))]);
                }
                command_output = output.to_string();
            } else {
                anyhow::bail!("Command '{}' not found", args.command);
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        Ok(vec![ToolResultContent::text(format!(
            "Command ID: {command_id}\nCommand is run\nOutput:\n{command_output}"
        ))])
    }
}

#[async_trait]
impl ToolImpl for GetCommandResultTool {
    fn desciption(&self) -> &serde_json::Value {
        &self.description
    }

    async fn call(&mut self, args: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<GetCommandResultToolArgs>(args)?;
        tracing::info!("Get command result '{}'", args.command_id);
        if let Some((exit_status, output)) = self
            .process_registry
            .read()
            .await
            .get_process(args.command_id)
        {
            Ok(vec![ToolResultContent::text(
                if let Some(exit_status) = exit_status {
                    format!(
                        "Command ID: {}\nExit Status: Exited({})\nOutput:\n{}",
                        args.command_id, exit_status, output
                    )
                } else {
                    format!(
                        "Command ID: {}\nCommand Still Running\nOutput:\n{}",
                        args.command_id, output
                    )
                },
            )])
        } else {
            anyhow::bail!("Command '{}' not found", args.command_id);
        }
    }
}

#[async_trait]
impl ToolImpl for TerminateCommandTool {
    fn desciption(&self) -> &serde_json::Value {
        &self.description
    }

    async fn call(&mut self, args: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<TerminateCommandToolArgs>(args)?;
        self.process_registry
            .write()
            .await
            .stop_process(args.command_id)?;
        Ok(vec![ToolResultContent::text(format!(
            "Command with ID {} successfully terminated.",
            args.command_id
        ))])
    }
}
