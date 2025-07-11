use std::sync::Arc;

use hulyrs::services::{
    transactor::TransactorClient,
    types::{AccountUuid, WorkspaceUuid},
};
use tokio::sync::RwLock;

use crate::tools::command::process_registry::ProcessRegistry;

pub struct AgentContext {
    pub social_id: String,
    pub tx_client: TransactorClient,
    pub process_registry: Arc<RwLock<ProcessRegistry>>,
}

pub struct MessagesContext {
    pub config: crate::config::Config,
    pub tx_client: TransactorClient,
    pub workspace_uuid: WorkspaceUuid,
    pub account_uuid: AccountUuid,
    pub person_id: String,
}
