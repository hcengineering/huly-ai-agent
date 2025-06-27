use std::str::FromStr;

use anyhow::{bail, Result};
use hulyrs::services::types::WorkspaceUuid;
use rdkafka::{
    consumer::{Consumer, StreamConsumer},
    message::BorrowedMessage,
    Message,
};
use tokio::sync::mpsc;

use crate::{
    context,
    huly::streaming::types::{
        CommunicationDomainEventKind, CreateMessage, DomainEventKind, StreamingMessage,
        StreamingMessageKind,
    },
};

pub mod types;

async fn process_message<'a>(
    message: &BorrowedMessage<'a>,
    match_pattern: &str,
) -> Result<Option<CreateMessage>> {
    let envelope = if let Some(payload) = message.payload() {
        match serde_json::from_slice::<serde_json::Value>(payload) {
            Ok(parsed) => parsed,
            Err(error) => {
                tracing::error!(%error);
                bail!("InvalidPayload");
            }
        }
    } else {
        bail!("NoPayload");
    };
    if !envelope.is_object() {
        bail!("InvalidPayload");
    }
    let message = serde_json::from_value::<StreamingMessage>(envelope)?;
    if let StreamingMessageKind::Domain(DomainEventKind::Communication(
        CommunicationDomainEventKind::CreateMessage(msg),
    )) = message.kind
    {
        if msg.content.contains(match_pattern) {
            return Ok(Some(msg));
        }
    }

    Ok(None)
}

fn to_kafka_log_level(level: tracing::Level) -> rdkafka::config::RDKafkaLogLevel {
    match level {
        tracing::Level::ERROR => rdkafka::config::RDKafkaLogLevel::Error,
        tracing::Level::WARN => rdkafka::config::RDKafkaLogLevel::Warning,
        tracing::Level::INFO => rdkafka::config::RDKafkaLogLevel::Info,
        tracing::Level::DEBUG => rdkafka::config::RDKafkaLogLevel::Debug,
        _ => rdkafka::config::RDKafkaLogLevel::Info,
    }
}

pub async fn worker(
    context: context::MessagesContext,
    sender: mpsc::UnboundedSender<CreateMessage>,
) -> Result<()> {
    let mut kafka_config = rdkafka::ClientConfig::new();
    kafka_config
        .set("group.id", context.config.huly.kafka.group_id)
        .set("bootstrap.servers", context.config.huly.kafka.bootstrap)
        .set_log_level(to_kafka_log_level(tracing::Level::from_str(
            &context.config.huly.kafka.log_level,
        )?));
    let consumer: StreamConsumer = kafka_config.create()?;
    let workspace_uuid = context.workspace_uuid;
    let topics = context
        .config
        .huly
        .kafka
        .topics
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>();

    tracing::info!(topics = %format!("[{}]", topics.join(",")), "Starting consumer");
    consumer.subscribe(&topics)?;
    let person_id = context.person_id.to_string();
    let match_pattern = format!("class%3APerson&_id={}", person_id);

    loop {
        let message = consumer.recv().await;

        let Ok(message) = message else {
            continue;
        };

        let Some(key) = message.key() else {
            continue;
        };
        let key = String::from_utf8_lossy(key);

        let Ok(workspace) = WorkspaceUuid::parse_str(&key) else {
            tracing::warn!(%key, "Invalid workspace UUID");
            continue;
        };

        if workspace != workspace_uuid {
            continue;
        }

        match process_message(&message, &match_pattern).await {
            Ok(Some(msg)) => {
                sender.send(msg)?;
            }
            Ok(None) => {}
            Err(error) => {
                tracing::error!(%error, "Error processing message");
            }
        }
    }
}
