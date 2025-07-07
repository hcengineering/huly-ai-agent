use hulyrs::services::{
    transactor::TransactorClient,
    types::{AccountUuid, WorkspaceUuid},
};

pub struct AgentContext {
    pub social_id: String,
    pub tx_client: TransactorClient,
}

pub struct MessagesContext {
    pub config: crate::config::Config,
    pub tx_client: TransactorClient,
    pub workspace_uuid: WorkspaceUuid,
    pub account_uuid: AccountUuid,
    pub person_id: String,
}
