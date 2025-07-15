use std::collections::HashMap;

use anyhow::{Result, bail};
use hulyrs::services::{
    transactor::document::{DocumentClient, FindOptionsBuilder},
    types::WorkspaceUuid,
};
use rdkafka::{
    Message,
    consumer::{Consumer, StreamConsumer},
    message::BorrowedMessage,
};
use tokio::sync::mpsc;

use crate::{
    context,
    huly::streaming::types::{
        CommunicationDomainEventKind, CreateMessage, DomainEventKind, StreamingMessage,
        StreamingMessageKind,
    },
    task::MAX_FOLLOW_MESSAGES,
};

pub mod types;

async fn process_message<'a>(
    message: &BorrowedMessage<'a>,
    match_pattern: &str,
    ignore_channel_id: &Option<String>,
    follow_channel_ids: &mut HashMap<String, u8>,
) -> Result<Option<(CreateMessage, bool)>> {
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
    //println!("msg: {}", serde_json::to_string_pretty(&envelope)?);
    let message = serde_json::from_value::<StreamingMessage>(envelope)?;
    if let StreamingMessageKind::Domain(DomainEventKind::Communication(
        CommunicationDomainEventKind::CreateMessage(msg),
    )) = message.kind
    {
        if ignore_channel_id
            .as_ref()
            .is_some_and(|channel_id| channel_id == &msg.card_id)
        {
            return Ok(None);
        }
        if msg.content.contains(match_pattern) {
            follow_channel_ids.insert(msg.card_id.clone(), MAX_FOLLOW_MESSAGES);
            return Ok(Some((msg, true)));
        } else if follow_channel_ids.contains_key(&msg.card_id) {
            let count = follow_channel_ids.get_mut(&msg.card_id).unwrap();
            *count = count.saturating_sub(1);
            if *count == 0 {
                follow_channel_ids.remove(&msg.card_id);
            }
            return Ok(Some((msg, false)));
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
        tracing::Level::TRACE => rdkafka::config::RDKafkaLogLevel::Debug,
    }
}

pub async fn worker(
    context: context::MessagesContext,
    sender: mpsc::UnboundedSender<(CreateMessage, bool)>,
) -> Result<()> {
    let mut kafka_config = rdkafka::ClientConfig::new();
    kafka_config
        .set("group.id", context.config.huly.kafka.group_id)
        .set("bootstrap.servers", context.config.huly.kafka.bootstrap)
        .set_log_level(to_kafka_log_level(context.config.log_level));
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
    let match_pattern = format!("class%3APerson&_id={person_id}");
    let ignore_channel_id = context.config.log_channel.clone();
    let mut follow_channel_ids = HashMap::<String, u8>::new();
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

        match process_message(
            &message,
            &match_pattern,
            &ignore_channel_id,
            &mut follow_channel_ids,
        )
        .await
        {
            Ok(Some((mut msg, is_mention))) => {
                let query = serde_json::json!({
                    "_id": msg.social_id,
                });
                let options = FindOptionsBuilder::default()
                    .project("attachedTo")
                    .build()?;

                if let Some(attached_to) = context
                    .tx_client
                    .find_one::<_, serde_json::Value>(
                        "contact:class:SocialIdentity",
                        query,
                        &options,
                    )
                    .await?
                {
                    let person_uuid = attached_to["attachedTo"].as_str().unwrap();
                    let query = serde_json::json!({
                        "_id": person_uuid,
                    });
                    let options = FindOptionsBuilder::default().project("name").build()?;
                    if let Some(person) = context
                        .tx_client
                        .find_one::<_, serde_json::Value>("contact:class:Person", query, &options)
                        .await?
                    {
                        msg.person_name = Some(person["name"].as_str().unwrap().to_string());
                    }
                };

                sender.send((msg, is_mention))?;
            }
            Ok(None) => {}
            Err(error) => {
                tracing::error!(%error, "Error processing message");
            }
        }
    }
}
