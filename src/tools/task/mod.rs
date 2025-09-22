// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;

use crate::{
    config::Config,
    context::AgentContext,
    state::AgentState,
    tools::{ToolImpl, ToolSet},
    types::ToolResultContent,
};

pub struct TaskToolSet;

impl ToolSet for TaskToolSet {
    fn get_name(&self) -> &str {
        "task"
    }

    async fn get_tools<'a>(
        &self,
        _config: &'a Config,
        _context: &'a AgentContext,
        _state: &'a AgentState,
    ) -> Vec<Box<dyn ToolImpl>> {
        let mut descriptions =
            serde_json::from_str::<Vec<serde_json::Value>>(include_str!("tools.json"))
                .unwrap()
                .into_iter()
                .map(|v| (v["function"]["name"].as_str().unwrap().to_string(), v))
                .collect::<HashMap<String, serde_json::Value>>();
        vec![
            Box::new(AddScheduledTaskTool {
                description: descriptions.remove("task_add_scheduled").unwrap(),
            }),
            Box::new(DeleteScheduledTaskTool {
                description: descriptions.remove("task_delete_scheduled").unwrap(),
            }),
        ]
    }

    fn get_system_prompt(&self, _config: &Config) -> String {
        include_str!("system_prompt.md").to_string()
    }

    async fn get_static_context(&self, _config: &Config) -> String {
        "${SCHEDULED_TASKS}".to_string()
    }
}

pub struct AddScheduledTaskTool {
    description: serde_json::Value,
}

#[derive(Deserialize)]
pub struct AddScheduledTaskToolArgs {
    pub content: String,
    pub schedule: String,
}

pub struct DeleteScheduledTaskTool {
    description: serde_json::Value,
}

#[derive(Deserialize)]
pub struct DeleteScheduledTaskToolArgs {
    pub id: i64,
}

#[async_trait]
impl ToolImpl for AddScheduledTaskTool {
    fn desciption(&self) -> &serde_json::Value {
        &self.description
    }

    async fn call(
        &mut self,
        context: &AgentContext,
        arguments: serde_json::Value,
    ) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<AddScheduledTaskToolArgs>(arguments)?;
        context
            .db_client
            .add_scheduled_task(&args.content, &args.schedule)
            .await?;
        Ok(vec![ToolResultContent::text(
            "Task added to the scheduler".to_string(),
        )])
    }
}

#[async_trait]
impl ToolImpl for DeleteScheduledTaskTool {
    fn desciption(&self) -> &serde_json::Value {
        &self.description
    }

    async fn call(
        &mut self,
        context: &AgentContext,
        arguments: serde_json::Value,
    ) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<DeleteScheduledTaskToolArgs>(arguments)?;
        context.db_client.delete_scheduled_task(args.id).await?;
        Ok(vec![ToolResultContent::text(
            "Task deleted from the scheduler".to_string(),
        )])
    }
}
