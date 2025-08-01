// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use anyhow::Result;
use async_trait::async_trait;
use hulyrs::services::transactor::{
    TransactorClient,
    backend::http::HttpBackend,
    comm::{CreateMessageEventBuilder, Envelope, MessageRequestType, MessageType},
};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    config::Config,
    context::AgentContext,
    state::AgentState,
    tools::{ToolImpl, ToolSet},
};

pub struct HulyToolSet;

impl ToolSet for HulyToolSet {
    fn get_tools<'a>(
        &self,
        _config: &'a Config,
        context: &'a AgentContext,
        _state: &'a AgentState,
    ) -> Vec<Box<dyn ToolImpl>> {
        vec![Box::new(SendMessageTool {
            social_id: context.social_id.clone(),
            tx_client: context.tx_client.clone(),
        })]
    }

    fn get_tool_descriptions(&self, _config: &Config) -> Vec<serde_json::Value> {
        serde_json::from_str(include_str!("tools.json")).unwrap()
    }

    fn get_system_prompt(&self, _config: &Config) -> String {
        include_str!("system_prompt.txt").to_string()
    }
}

struct SendMessageTool {
    social_id: String,
    tx_client: TransactorClient<HttpBackend>,
}

#[derive(Deserialize)]
struct SendMessageToolArgs {
    channel: String,
    content: String,
}

#[async_trait]
impl ToolImpl for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }

    async fn call(&mut self, args: serde_json::Value) -> Result<String> {
        let args = serde_json::from_value::<SendMessageToolArgs>(args)?;
        tracing::debug!(
            channel = args.channel,
            content = args.content,
            "Send message to channel"
        );
        let card_id = args.channel;

        let create_event = CreateMessageEventBuilder::default()
            .message_type(MessageType::Message)
            .card_id(card_id)
            .card_type("chat:masterTag:Channel")
            .content(args.content)
            .social_id(&self.social_id)
            .build()
            .unwrap();

        let create_event = Envelope::new(MessageRequestType::CreateMessage, create_event);

        let res = self.tx_client.tx::<_, Value>(create_event).await?;
        Ok(format!("Message sent, message_id is {}", res["messageId"]))
    }
}
