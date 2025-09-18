// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{collections::HashMap, sync::Arc};

use hulyrs::services::{
    core::{AccountUuid, PersonUuid, WorkspaceUuid},
    transactor::{TransactorClient, backend::http::HttpBackend},
};
use secrecy::SecretString;
use tokio::sync::RwLock;

use crate::{
    huly::{blob::BlobClient, streaming::types::PersonInfo, typing::TypingClient},
    tools::command::process_registry::ProcessRegistry,
};

pub struct AgentContext {
    pub account_info: HulyAccountInfo,
    pub tx_client: TransactorClient<HttpBackend>,
    pub blob_client: BlobClient,
    pub typing_client: TypingClient,
    pub process_registry: Arc<RwLock<ProcessRegistry>>,
    pub channel_log_writer: Option<crate::channel_log::HulyChannelLogWriter>,
    pub db_client: crate::database::DbClient,
    pub tools_context: Option<String>,
    pub tools_system_prompt: Option<String>,
}

#[derive(Debug, Clone)]
pub struct HulyAccountInfo {
    pub account_uuid: PersonUuid,
    pub person_name: String,
    pub token: SecretString,
    pub social_id: String,
    pub person_id: String,
    pub workspace: WorkspaceUuid,
    pub control_card_id: Option<String>,
}

#[derive(Clone)]
pub struct CardInfo {
    pub title: String,
    pub space: String,
    pub parent: Option<String>,
}

#[derive(Clone)]
pub struct SpaceInfo {
    pub can_read: bool,
    pub is_personal: bool,
}

pub struct MessagesContext {
    pub config: crate::config::Config,
    pub server_config: crate::huly::ServerConfig,
    pub tx_client: TransactorClient<HttpBackend>,
    pub workspace_uuid: WorkspaceUuid,
    pub account_uuid: AccountUuid,
    pub person_id: String,
    pub card_info_cache: HashMap<String, CardInfo>,
    pub space_info_cache: HashMap<String, SpaceInfo>,
    pub person_info_cache: HashMap<String, PersonInfo>,
}
