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

pub struct NotesToolSet;

impl ToolSet for NotesToolSet {
    fn get_name(&self) -> &str {
        "notes"
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
            Box::new(AddNoteTool {
                description: descriptions.remove("notes_add").unwrap(),
            }),
            Box::new(DeleteNotesTool {
                description: descriptions.remove("notes_delete").unwrap(),
            }),
        ]
    }

    fn get_system_prompt(&self, _config: &Config) -> String {
        include_str!("system_prompt.md").to_string()
    }

    async fn get_static_context(&self, _config: &Config) -> String {
        "${NOTES}".to_string()
    }
}

pub struct AddNoteTool {
    description: serde_json::Value,
}

#[derive(Deserialize)]
pub struct AddNoteToolArgs {
    pub content: String,
}

pub struct DeleteNotesTool {
    description: serde_json::Value,
}

#[derive(Deserialize)]
pub struct DeleteNotesToolArgs {
    pub ids: String,
}

#[async_trait]
impl ToolImpl for AddNoteTool {
    fn desciption(&self) -> &serde_json::Value {
        &self.description
    }

    async fn call(
        &mut self,
        context: &AgentContext,
        arguments: serde_json::Value,
    ) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<AddNoteToolArgs>(arguments)?;
        let id = context.db_client.add_note(&args.content).await?;
        Ok(vec![ToolResultContent::text(format!(
            "Note added with id {}",
            id
        ))])
    }
}

#[async_trait]
impl ToolImpl for DeleteNotesTool {
    fn desciption(&self) -> &serde_json::Value {
        &self.description
    }

    async fn call(
        &mut self,
        context: &AgentContext,
        arguments: serde_json::Value,
    ) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<DeleteNotesToolArgs>(arguments)?;
        let Ok(ids) = args
            .ids
            .split(',')
            .map(|s| s.parse::<i64>())
            .collect::<Result<Vec<i64>, _>>()
        else {
            anyhow::bail!("Invalid ids");
        };
        context.db_client.delete_notes(ids).await?;
        Ok(vec![ToolResultContent::text(
            "Notes deleted from the notes".to_string(),
        )])
    }
}
