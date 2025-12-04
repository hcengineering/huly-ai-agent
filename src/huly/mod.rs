// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use anyhow::Result;
use hulyrs::services::transactor::{
    TransactorClient,
    backend::http::HttpBackend,
    comm::{
        CreateMessageEventBuilder, Envelope, MessageRequestType, MessageType,
        ReactionPatchEventBuilder, ReactionPatchOperation,
    },
};
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;

pub mod blob;
pub mod types;
pub mod typing;

#[derive(Deserialize, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[allow(dead_code)]
pub struct ServerConfig {
    pub accounts_url: Url,
    pub hulylake_url: Option<String>,
    pub datalake_url: Option<String>,
    pub files_url: String,
    pub pulse_url: Url,
    pub collaborator_url: Url,
}

pub async fn fetch_server_config(base_url: Url) -> Result<ServerConfig> {
    Ok(reqwest::get(base_url.join("/config.json")?)
        .await?
        .json()
        .await?)
}

pub async fn add_reaction(
    tx_client: &TransactorClient<HttpBackend>,
    card_id: &str,
    message_id: &str,
    social_id: &str,
    reaction: &str,
) -> Result<()> {
    let reaction_event = ReactionPatchEventBuilder::default()
        .card_id(card_id)
        .message_id(message_id)
        .operation(ReactionPatchOperation::Add {
            reaction: reaction.to_string(),
        })
        .social_id(social_id)
        .build()
        .unwrap();

    let add_reaction = Envelope::new(MessageRequestType::ReactionPatch, reaction_event);

    if message_id.is_empty() {
        tracing::warn!("Message ID is empty");
        return Ok(());
    }
    tx_client.tx::<_, Value>(add_reaction).await?;
    Ok(())
}

pub async fn send_message(
    tx_client: &TransactorClient<HttpBackend>,
    card_id: &str,
    social_id: &str,
    content: &str,
) -> Result<String> {
    let create_event = CreateMessageEventBuilder::default()
        .message_type(MessageType::Text)
        .card_id(card_id)
        .card_type("chat:masterTag:Thread")
        .content(content)
        .social_id(social_id)
        .build()
        .unwrap();

    let create_event = Envelope::new(MessageRequestType::CreateMessage, create_event);

    let res = tx_client.tx::<_, Value>(create_event).await?;
    Ok(res["messageId"].as_str().unwrap_or_default().to_string())
}
