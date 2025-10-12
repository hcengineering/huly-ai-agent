// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use hulyrs::services::{
    event::Class,
    transactor::{
        TransactorClient,
        backend::http::HttpBackend,
        document::{DocumentClient, FindOptionsBuilder},
    },
};
use serde_json::json;
use std::collections::HashSet;
use streaming::{AgentInfo, types::CommunicationEvent};
use tokio::{select, sync::mpsc};

use crate::{
    config::Config,
    context::HulyAccountInfo,
    huly::{ServerConfig, types::CommunicationDirect},
};

async fn event_to_http_processor(
    social_id: String,
    mut messages_receiver: mpsc::UnboundedReceiver<(
        HashSet<std::string::String>,
        CommunicationEvent,
    )>,
) {
    let http_client = reqwest::Client::new();

    while let Some((recipients, event)) = messages_receiver.recv().await {
        let recipients = recipients.into_iter().collect::<Vec<_>>();
        if recipients.is_empty() {
            tracing::warn!("Empty recipients");
            continue;
        }
        if recipients.len() > 1 {
            tracing::warn!("Multiple recipients");
            continue;
        }

        if !recipients.contains(&social_id) {
            tracing::warn!("Incorrect recipient");
            continue;
        }

        http_client
            .post("http://localhost:8081/event")
            .json(&event)
            .send()
            .await
            .unwrap();
    }
}

pub async fn streaming_worker(
    config: &Config,
    server_config: &ServerConfig,
    account_info: HulyAccountInfo,
    tx_client: TransactorClient<HttpBackend>,
) {
    let (agent_info_tx, agent_info_rx) = mpsc::unbounded_channel();
    let (comm_messages_sender, comm_messages_receiver) = mpsc::unbounded_channel();

    let direct_cards = tx_client
        .find_all::<_, serde_json::Value>(
            CommunicationDirect::CLASS,
            json!({}),
            &FindOptionsBuilder::default().project("_id").build(),
        )
        .await
        .unwrap()
        .value
        .iter()
        .map(|card| card["_id"].as_str().unwrap().to_string())
        .collect::<HashSet<String>>();

    agent_info_tx
        .send(vec![AgentInfo {
            workspace_uuid: account_info.workspace,
            account_uuid: account_info.account_uuid,
            social_id: account_info.social_id.clone(),
            persistent_cards: direct_cards,
            tx_client,
        }])
        .unwrap();

    let kafka_config = serde_json::from_value(config.huly.kafka.clone()).unwrap();
    select! {
        _ = streaming::worker(
            &kafka_config,
            agent_info_rx,
            comm_messages_sender,
            &server_config.files_url,
            config.log_level,
        ) => {},
        _ = event_to_http_processor(account_info.social_id.clone(), comm_messages_receiver) => {

        }
    }
}
