// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use anyhow::Result;
use async_trait::async_trait;
use secrecy::ExposeSecret;

use crate::{
    config::{Config, ProviderKind},
    types::{Message, streaming::StreamingCompletionResponse},
};

mod openrouter;

#[async_trait]
pub trait ProviderClient: Send + Sync {
    /// Sends messages to the provider and returns a streaming response.
    /// The system prompt and context are used to provide additional information to the provider.
    async fn send_messages(
        &self,
        system_prompt: &str,
        context: &str,
        messages: &[Message],
    ) -> Result<StreamingCompletionResponse>;
}

pub fn create_provider_client(
    config: &Config,
    tools: Vec<serde_json::Value>,
) -> Result<Box<dyn ProviderClient>> {
    match config.provider {
        ProviderKind::OpenRouter => Ok(Box::new(openrouter::Client::new(
            config.provider_api_key.as_ref().unwrap().expose_secret(),
            &config.model,
            tools,
        )?)),
        _ => Err(anyhow::anyhow!("Unsupported provider")),
    }
}
