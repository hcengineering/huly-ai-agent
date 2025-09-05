// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use mcp_core::{client::Client, transport::Transport};

use crate::{
    tools::ToolImpl,
    types::{ImageMediaType, ToolResultContent},
};

pub struct McpTool<T: Transport> {
    client: Arc<Client<T>>,
    desciption: serde_json::Value,
}

impl<T: Transport> McpTool<T> {
    pub fn new(client: Arc<Client<T>>, desciption: serde_json::Value) -> Self {
        Self { client, desciption }
    }
}

#[async_trait]
impl<T: Transport> ToolImpl for McpTool<T> {
    fn desciption(&self) -> &serde_json::Value {
        &self.desciption
    }

    async fn call(&mut self, arguments: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        tracing::trace!(
            tool = self.name(),
            args = arguments.to_string(),
            "mcp_tool call"
        );
        let res = self
            .client
            .call_tool(
                self.name(),
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
                    Some(ToolResultContent::text(text_content.text.clone()))
                }
                mcp_core::types::ToolResponseContent::Image(image) => {
                    Some(ToolResultContent::image(
                        image.data.clone(),
                        ImageMediaType::from_mime_type(&image.mime_type),
                    ))
                }
                mcp_core::types::ToolResponseContent::Audio(_) => None,
                mcp_core::types::ToolResponseContent::Resource(_) => None,
            })
            .collect::<Vec<_>>();
        Ok(res)
    }
}
