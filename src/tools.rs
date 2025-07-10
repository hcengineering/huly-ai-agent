// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use anyhow::Result;
use async_trait::async_trait;

use crate::{config::Config, context::AgentContext, state::AgentState};

pub mod files;
pub mod huly;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod memory;

#[async_trait]
pub trait ToolImpl: Send + Sync {
    fn name(&self) -> &str;
    async fn call(&mut self, arguments: serde_json::Value) -> Result<String>;
}

pub trait ToolSet {
    fn get_tools<'a>(
        config: &'a Config,
        context: &'a AgentContext,
        state: &'a AgentState,
    ) -> Vec<Box<dyn ToolImpl>>;
    fn get_tool_descriptions() -> Vec<serde_json::Value>;
    fn get_system_prompt<'a>() -> &'a str;
}
