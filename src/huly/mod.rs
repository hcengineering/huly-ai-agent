// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use anyhow::Result;
use hulyrs::services::transactor::{
    TransactorClient,
    backend::http::HttpBackend,
    comm::{Envelope, MessageRequestType, ReactionPatchEventBuilder, ReactionPatchOperation},
};
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;

pub mod blob;
pub mod streaming;
pub mod types;

#[derive(Deserialize, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct ServerConfig {
    pub accounts_url: Url,
    pub upload_url: String,
    pub files_url: String,
}

pub async fn fetch_server_config(base_url: Url) -> Result<ServerConfig> {
    Ok(reqwest::get(base_url.join("/config.json")?)
        .await?
        .json()
        .await?)
}

pub async fn add_reaction(
    tx_client: &TransactorClient<HttpBackend>,
    channel_id: &str,
    message_id: &str,
    social_id: &str,
    reaction: &str,
) -> Result<()> {
    let reaction_event = ReactionPatchEventBuilder::default()
        .card_id(channel_id)
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
