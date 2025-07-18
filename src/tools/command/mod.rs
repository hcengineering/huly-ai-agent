// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
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
};

pub mod process_registry;

const COMMAND_TIMEOUT: u64 = 300; // 30 secs

pub struct CommandsToolSet;

impl ToolSet for CommandsToolSet {
    fn get_tools<'a>(
        &self,
        config: &'a Config,
        context: &'a AgentContext,
        _state: &'a AgentState,
    ) -> Vec<Box<dyn ToolImpl>> {
        vec![
            Box::new(ExecuteCommandTool {
                workspace: config.workspace.clone(),
                process_registry: context.process_registry.clone(),
            }),
            Box::new(GetCommandResultTool {
                process_registry: context.process_registry.clone(),
            }),
            Box::new(TerminateCommandTool {
                process_registry: context.process_registry.clone(),
            }),
        ]
    }

    fn get_tool_descriptions(&self, config: &Config) -> Vec<serde_json::Value> {
        serde_json::from_str(
            &include_str!("tools.json")
                .replace("${WORKSPACE}", &workspace_to_string(&config.workspace)),
        )
        .unwrap()
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct TerminateCommandToolArgs {
    pub command_id: usize,
}

pub struct TerminateCommandTool {
    process_registry: Arc<RwLock<ProcessRegistry>>,
}

#[async_trait]
impl ToolImpl for ExecuteCommandTool {
    fn name(&self) -> &str {
        "execute_command"
    }

    async fn call(&mut self, args: serde_json::Value) -> Result<String> {
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
                    return Ok(format!(
                        "Command ID: {command_id}\nExit Status: Exited({exit_status})\nOutput:\n{output}"
                    ));
                }
                command_output = output.to_string();
            } else {
                anyhow::bail!("Command '{}' not found", args.command);
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        Ok(format!(
            "Command ID: {command_id}\nCommand is run\nOutput:\n{command_output}"
        ))
    }
}

#[async_trait]
impl ToolImpl for GetCommandResultTool {
    fn name(&self) -> &str {
        "terminate_command"
    }

    async fn call(&mut self, args: serde_json::Value) -> Result<String> {
        let args = serde_json::from_value::<GetCommandResultToolArgs>(args)?;
        tracing::info!("Get command result '{}'", args.command_id);
        if let Some((exit_status, output)) = self
            .process_registry
            .read()
            .await
            .get_process(args.command_id)
        {
            if let Some(exit_status) = exit_status {
                Ok(format!(
                    "Command ID: {}\nExit Status: Exited({})\nOutput:\n{}",
                    args.command_id, exit_status, output
                ))
            } else {
                Ok(format!(
                    "Command ID: {}\nCommand Still Running\nOutput:\n{}",
                    args.command_id, output
                ))
            }
        } else {
            anyhow::bail!("Command '{}' not found", args.command_id);
        }
    }
}

#[async_trait]
impl ToolImpl for TerminateCommandTool {
    fn name(&self) -> &str {
        "get_command_result"
    }

    async fn call(&mut self, args: serde_json::Value) -> Result<String> {
        let args = serde_json::from_value::<TerminateCommandToolArgs>(args)?;
        self.process_registry
            .write()
            .await
            .stop_process(args.command_id)?;
        Ok(format!(
            "Command with ID {} successfully terminated.",
            args.command_id
        ))
    }
}
