use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use hulyrs::services::transactor::{
    self,
    document::{DocumentClient, FindOptionsBuilder},
};
use rdkafka::consumer::{Consumer, StreamConsumer};
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

fn try_extract_create_message_from_payload(
    payload: serde_json::Value,
) -> Result<Option<CreateMessage>> {
    if !payload.is_object() {
        bail!("InvalidPayload");
    }
    let message = serde_json::from_value::<StreamingMessage>(payload)?;
    if let StreamingMessageKind::Domain(DomainEventKind::Communication(
        CommunicationDomainEventKind::CreateMessage(msg),
    )) = message.kind
    {
        return Ok(Some(msg));
    }
    Ok(None)
}

fn should_process_message(
    msg: &CreateMessage,
    match_pattern: &str,
    ignore_channel_id: &Option<String>,
    follow_channel_ids: &mut HashMap<String, u8>,
) -> Option<bool> {
    if ignore_channel_id
        .as_ref()
        .is_some_and(|channel_id| channel_id == &msg.card_id)
    {
        return None;
    }
    if msg.content.contains(match_pattern) {
        follow_channel_ids.insert(msg.card_id.clone(), MAX_FOLLOW_MESSAGES);
        return Some(true);
    } else if let Some(count) = follow_channel_ids.get_mut(&msg.card_id) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            follow_channel_ids.remove(&msg.card_id);
        }
        return Some(false);
    }

    None
}

async fn enrich_create_message(
    context: &context::MessagesContext,
    mut msg: CreateMessage,
) -> Result<CreateMessage> {
    let query = serde_json::json!({
        "_id": msg.social_id,
    });
    let options = FindOptionsBuilder::default()
        .project("attachedTo")
        .build()?;

    if let Some(attached_to) = context
        .tx_client
        .find_one::<_, serde_json::Value>("contact:class:SocialIdentity", query, &options)
        .await?
    {
        let person_uuid = attached_to["attachedTo"]
            .as_str()
            .context("missing attachedTo field")?;
        msg.person_id = Some(person_uuid.to_string());
        let query = serde_json::json!({
            "_id": person_uuid,
        });
        let options = FindOptionsBuilder::default().project("name").build()?;
        if let Some(person) = context
            .tx_client
            .find_one::<_, serde_json::Value>("contact:class:Person", query, &options)
            .await?
        {
            msg.person_name = Some(
                person["name"]
                    .as_str()
                    .context("missing name field")?
                    .to_string(),
            );
        }
    };
    Ok(msg)
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
        .set("group.id", &context.config.huly.kafka.group_id)
        .set("bootstrap.servers", &context.config.huly.kafka.bootstrap)
        .set_log_level(to_kafka_log_level(context.config.log_level));
    let consumer: StreamConsumer = kafka_config.create()?;
    let listening_workspace_uuid = context.workspace_uuid;
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
    let match_pattern = format!("ref://?_class=contact%3Aclass%3APerson&_id={person_id}");
    let ignore_channel_id = context.config.log_channel.clone();
    let mut follow_channel_ids = HashMap::<String, u8>::new();
    loop {
        let Ok(kafka_message) = consumer.recv().await else {
            continue;
        };
        let (workspace, transactor_payload) = match transactor::kafka::parse_message(&kafka_message)
        {
            Ok(data) => data,
            Err(err) => {
                tracing::trace!(%err, "Unknown message format, skipping");
                continue;
            }
        };
        if workspace != listening_workspace_uuid {
            continue;
        }

        let message = match try_extract_create_message_from_payload(transactor_payload) {
            Ok(Some(m)) => m,
            Ok(None) => {
                continue;
            }
            Err(error) => {
                tracing::error!(%error, "Error parsing message from queue");
                continue;
            }
        };

        let Some(is_mention) = should_process_message(
            &message,
            &match_pattern,
            &ignore_channel_id,
            &mut follow_channel_ids,
        ) else {
            continue;
        };
        let message = enrich_create_message(&context, message).await?;

        sender.send((message, is_mention))?;
    }
}
