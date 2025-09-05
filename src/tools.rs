// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use anyhow::Result;
use async_trait::async_trait;

use crate::{config::Config, context::AgentContext, state::AgentState, types::ToolResultContent};

pub mod browser;
pub mod command;
pub mod files;
pub mod huly;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod web;

#[async_trait]
pub trait ToolImpl: Send + Sync {
    fn name(&self) -> &str {
        self.desciption()
            .get("function")
            .unwrap()
            .get("name")
            .unwrap()
            .as_str()
            .unwrap()
    }

    async fn call(&mut self, arguments: serde_json::Value) -> Result<Vec<ToolResultContent>>;
    fn desciption(&self) -> &serde_json::Value;
}

pub trait ToolSet {
    fn get_name(&self) -> &str;
    async fn get_tools<'a>(
        &self,
        config: &'a Config,
        context: &'a AgentContext,
        state: &'a AgentState,
    ) -> Vec<Box<dyn ToolImpl>>;
    fn get_system_prompt(&self, config: &Config) -> String;
    async fn get_context(&self, _config: &Config) -> String {
        "".to_string()
    }
}
