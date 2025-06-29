// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use mcp_core::{client::Client, transport::Transport};

use crate::tools::ToolImpl;

pub struct McpTool<T: Transport> {
    name: String,
    client: Arc<Client<T>>,
}

impl<T: Transport> McpTool<T> {
    pub fn new(name: String, client: Arc<Client<T>>) -> Self {
        Self { name, client }
    }
}

#[async_trait]
impl<T: Transport> ToolImpl for McpTool<T> {
    fn name(&self) -> &str {
        self.name.as_str()
    }

    async fn call(&self, arguments: serde_json::Value) -> Result<String> {
        tracing::trace!(
            tool = self.name,
            args = arguments.to_string(),
            "mcp_tool call"
        );
        let res = self
            .client
            .call_tool(
                &self.name,
                if arguments.is_null() {
                    None
                } else {
                    Some(arguments)
                },
            )
            .await?;
        tracing::trace!(result = ?res, "mcp_tool_result");
        let res = res
            .content
            .iter()
            .filter_map(|c| match c {
                mcp_core::types::ToolResponseContent::Text(text_content) => {
                    Some(text_content.text.clone())
                }
                mcp_core::types::ToolResponseContent::Image(_) => None,
                mcp_core::types::ToolResponseContent::Audio(_) => None,
                mcp_core::types::ToolResponseContent::Resource(_) => None,
            })
            .collect::<Vec<_>>();
        Ok(res.join("\n"))
    }
}
