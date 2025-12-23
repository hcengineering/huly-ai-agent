// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use hulyrs::services::transactor::{TransactorClient, backend::http::HttpBackend};
use serde::Serialize;

use crate::{config::Config, context::HulyAccountInfo, huly::ServerConfig};

pub mod http;
#[cfg(feature = "streaming")]
mod streaming;
pub mod types;

#[derive(Debug, Serialize)]
pub struct ScheduledTask {
    pub task_kind: String,
    pub schedule: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct AgentState {
    pub has_actve_task: bool,
    pub next_scheduled: Option<ScheduledTask>,
}

#[cfg(feature = "streaming")]
pub async fn streaming_worker(
    config: &Config,
    server_config: &ServerConfig,
    account_info: HulyAccountInfo,
    tx_client: TransactorClient<HttpBackend>,
) {
    streaming::streaming_worker(config, server_config, account_info, tx_client).await
}

#[cfg(not(feature = "streaming"))]
pub async fn streaming_worker(
    _config: &Config,
    _server_config: &ServerConfig,
    _account_info: HulyAccountInfo,
    _tx_client: TransactorClient<HttpBackend>,
) {
    std::future::pending().await
}
