use std::{collections::HashMap, sync::Arc};

use hulyrs::services::{
    core::{AccountUuid, WorkspaceUuid},
    transactor::{TransactorClient, backend::http::HttpBackend},
};
use tokio::sync::RwLock;

use crate::{
    huly::{blob::BlobClient, streaming::types::PersonInfo},
    tools::command::process_registry::ProcessRegistry,
};

pub struct AgentContext {
    pub social_id: String,
    pub tx_client: TransactorClient<HttpBackend>,
    pub blob_client: BlobClient,
    pub process_registry: Arc<RwLock<ProcessRegistry>>,
}

pub struct MessagesContext {
    pub config: crate::config::Config,
    pub server_config: crate::huly::ServerConfig,
    pub tx_client: TransactorClient<HttpBackend>,
    pub workspace_uuid: WorkspaceUuid,
    pub account_uuid: AccountUuid,
    pub person_id: String,
    pub channel_titles_cache: HashMap<String, String>,
    pub person_info_cache: HashMap<String, PersonInfo>,
}
