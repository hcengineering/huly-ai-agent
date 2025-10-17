// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::sync::Arc;

use hulyrs::services::{
    core::{PersonUuid, WorkspaceUuid},
    transactor::{TransactorClient, backend::http::HttpBackend},
};
use secrecy::SecretString;
use tokio::sync::RwLock;

use crate::{
    huly::{blob::BlobClient, typing::TypingClient},
    tools::command::process_registry::ProcessRegistry,
};

pub struct AgentContext {
    pub account_info: HulyAccountInfo,
    pub tx_client: TransactorClient<HttpBackend>,
    pub blob_client: BlobClient,
    pub typing_client: TypingClient,
    pub process_registry: Arc<RwLock<ProcessRegistry>>,
    pub db_client: crate::database::DbClient,
    pub tools_context: Option<String>,
    pub tools_system_prompt: Option<String>,
}

#[derive(Debug, Clone)]
pub struct HulyAccountInfo {
    pub account_uuid: PersonUuid,
    pub person_name: String,
    pub token: SecretString,
    #[allow(dead_code)]
    pub main_social_id: Option<String>,
    pub social_id: String,
    pub person_id: String,
    pub workspace: WorkspaceUuid,
    pub control_card_id: Option<String>,
    pub time_zone: chrono_tz::Tz,
}
