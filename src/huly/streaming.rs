// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, bail};
use hulyrs::services::{
    card,
    core::{Space, storage::WithoutStructure},
    transactor::{
        self,
        document::{DocumentClient, FindOptionsBuilder},
    },
};
use percent_encoding::NON_ALPHANUMERIC;
use rdkafka::consumer::{Consumer, StreamConsumer};
use tokio::sync::mpsc;
use types::{MessageType, ReceivedMessage, ThreadPatchOperation};

use crate::{
    context::{self, CardInfo, SpaceInfo},
    huly::streaming::types::{
        CommunicationDomainEventKind, CommunicationEvent, CreateMessage, DomainEventKind,
        PersonInfo, ReceivedAttachment, ReceivedReaction, StreamingMessage, StreamingMessageKind,
    },
    task::MAX_FOLLOW_MESSAGES,
};

use super::types::{Person, SocialIdentity};

pub mod types;

fn try_extract_communication_event_from_payload(
    payload: serde_json::Value,
) -> Result<Option<CommunicationDomainEventKind>> {
    if !payload.is_object() {
        bail!("InvalidPayload");
    }
    let message = serde_json::from_value::<StreamingMessage>(payload)?;
    if let StreamingMessageKind::Domain(DomainEventKind::Communication(event)) = message.kind {
        return Ok(Some(event));
    }
    Ok(None)
}

async fn get_card_info(context: &mut context::MessagesContext, card_id: &str) -> Result<CardInfo> {
    if let Some(card_info) = context.card_info_cache.get(card_id) {
        return Ok(card_info.clone());
    };
    let options = FindOptionsBuilder::default()
        .project("title")
        .project("space")
        .project("parent")
        .build();
    let query = serde_json::json!({
        "_id": card_id,
    });
    if let Some(card) = context
        .tx_client
        .find_one::<WithoutStructure<card::Card>, serde_json::Value>(query, &options)
        .await?
    {
        let card_title = card.data["title"]
            .as_str()
            .context("missing title field")?
            .to_string();
        let card_space = card.data["space"]
            .as_str()
            .context("missing space field")?
            .to_string();
        let card_info = CardInfo {
            title: card_title,
            space: card_space,
            parent: card.data["parent"].as_str().map(|s| s.to_string()),
        };
        context
            .card_info_cache
            .insert(card_id.to_string(), card_info.clone());
        Ok(card_info)
    } else {
        bail!("Failed to get card info");
    }
}

async fn get_space_info(
    context: &mut context::MessagesContext,
    space_id: &str,
) -> Result<SpaceInfo> {
    if let Some(space_info) = context.space_info_cache.get(space_id) {
        return Ok(space_info.clone());
    };
    let options = FindOptionsBuilder::default().project("_class").build();
    let query = serde_json::json!({
        "_id": space_id,
        "members": {
            "$in": &[&context.account_uuid]
        }
    });
    let Some(space) = context
        .tx_client
        .find_one::<WithoutStructure<Space>, serde_json::Value>(query, &options)
        .await?
    else {
        context.space_info_cache.insert(
            space_id.to_string(),
            SpaceInfo {
                can_read: false,
                is_personal: false,
            },
        );
        return Ok(SpaceInfo {
            can_read: false,
            is_personal: false,
        });
    };
    let space_class = space.data["_class"].as_str().unwrap_or_default();
    let is_personal = space_class == "contact:class:PersonSpace";

    context.space_info_cache.insert(
        space_id.to_string(),
        SpaceInfo {
            can_read: true,
            is_personal,
        },
    );
    Ok(SpaceInfo {
        can_read: true,
        is_personal,
    })
}

async fn should_process_message(
    context: &mut context::MessagesContext,
    msg: &CreateMessage,
    match_pattern: &str,
    ignore_card_ids: &HashSet<String>,
    follow_card_ids: &mut HashMap<String, u8>,
    persistent_cards: &mut HashSet<String>,
) -> Option<bool> {
    let card_info = get_card_info(context, &msg.card_id).await.ok()?;
    let space_info = get_space_info(context, &card_info.space).await.ok()?;
    if !space_info.can_read || ignore_card_ids.contains(&msg.card_id) {
        return None;
    }
    if persistent_cards.contains(&msg.card_id)
        || card_info
            .parent
            .is_some_and(|parent_id| persistent_cards.contains(&parent_id))
    {
        return Some(false);
    }

    if msg.message_type == MessageType::Message && msg.content.contains(match_pattern) {
        follow_card_ids.insert(msg.card_id.clone(), MAX_FOLLOW_MESSAGES);
        return Some(true);
    } else if space_info.can_read && space_info.is_personal {
        // Nobody else can read messages from personal space, meaning it is direct-like message
        if follow_card_ids.contains_key(&msg.card_id) {
            return Some(false);
        } else {
            follow_card_ids.insert(msg.card_id.clone(), MAX_FOLLOW_MESSAGES);
            return Some(true);
        }
    } else if let Some(count) = follow_card_ids.get_mut(&msg.card_id) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            follow_card_ids.remove(&msg.card_id);
        }
        return Some(false);
    }

    None
}

async fn enrich_create_message(
    context: &mut context::MessagesContext,
    msg: CreateMessage,
    is_mention: bool,
) -> Result<ReceivedMessage> {
    let mut msg = ReceivedMessage::from(msg);
    let card_info = get_card_info(context, &msg.card_id).await?;
    msg.person_info = get_person_info(context, &msg.social_id).await?;
    msg.parent_id = card_info.parent;
    msg.is_mention = is_mention;
    msg.card_title = Some(card_info.title);
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

async fn get_person_info(
    context: &mut context::MessagesContext,
    social_id: &str,
) -> Result<PersonInfo> {
    if let Some(person_info) = context.person_info_cache.get(social_id) {
        return Ok(person_info.clone());
    }
    let query = serde_json::json!({
        "_id": social_id,
    });
    let options = FindOptionsBuilder::default().project("attachedTo").build();

    let mut person_id = String::new();
    let mut person_name = String::new();

    if let Some(attached_to) = context
        .tx_client
        .find_one::<WithoutStructure<SocialIdentity>, serde_json::Value>(query, &options)
        .await?
    {
        let person_uuid = attached_to.data["attachedTo"]
            .as_str()
            .context("missing attachedTo field")?;
        person_id = person_uuid.to_string();
        let query = serde_json::json!({
            "_id": person_uuid,
        });
        let options = FindOptionsBuilder::default().project("name").build();
        if let Some(person) = context
            .tx_client
            .find_one::<WithoutStructure<Person>, serde_json::Value>(query, &options)
            .await?
        {
            person_name = person.data["name"]
                .as_str()
                .context("missing name field")?
                .to_string();
        }
    };
    let person_info = PersonInfo {
        person_id,
        person_name,
    };
    context
        .person_info_cache
        .insert(social_id.to_string(), person_info.clone());
    Ok(person_info)
}

pub async fn worker(
    mut context: context::MessagesContext,
    sender: mpsc::UnboundedSender<CommunicationEvent>,
    persistent_cards: HashSet<String>,
) -> Result<()> {
    let mut kafka_config = rdkafka::ClientConfig::new();
    kafka_config
        .set("group.id", &context.config.huly.kafka.group_id)
        .set("bootstrap.servers", &context.config.huly.kafka.bootstrap)
        .set_log_level(to_kafka_log_level(context.config.log_level));
    let consumer: StreamConsumer = kafka_config.create()?;
    let listening_workspace_uuid = context.workspace_uuid;

    tracing::info!(topics = %format!("[{}]", context.config.huly.kafka.topics.transactions), "Starting consumer");
    consumer.subscribe(&[&context.config.huly.kafka.topics.transactions])?;
    let person_id = context.person_id.to_string();
    let match_pattern = format!("ref://?_class=contact%3Aclass%3APerson&_id={person_id}");
    let ignore_card_ids = context.config.huly.ignored_channels.clone();
    let mut persistent_cards = persistent_cards.clone();
    let mut follow_card_ids = HashMap::<String, u8>::new();
    let mut tracked_message_ids = HashSet::<String>::new();

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

        let event = match try_extract_communication_event_from_payload(transactor_payload) {
            Ok(Some(e)) => e,
            Ok(None) => {
                continue;
            }
            Err(error) => {
                tracing::error!(%error, "Error parsing message from queue");
                continue;
            }
        };

        match event {
            CommunicationDomainEventKind::CreateMessage(message) => {
                let Some(is_mention) = should_process_message(
                    &mut context,
                    &message,
                    &match_pattern,
                    &ignore_card_ids,
                    &mut follow_card_ids,
                    &mut persistent_cards,
                )
                .await
                else {
                    continue;
                };
                tracked_message_ids.insert(message.message_id.clone());
                let message = enrich_create_message(&mut context, message, is_mention).await?;
                sender.send(CommunicationEvent::Message(message))?;
            }
            CommunicationDomainEventKind::AttachmentPatch(patch) => {
                if tracked_message_ids.contains(&patch.message_id) {
                    for attachement in patch
                        .operations
                        .iter()
                        .filter_map(|op| {
                            if op.opcode == "add" {
                                Some(&op.attachments)
                            } else {
                                None
                            }
                        })
                        .flatten()
                    {
                        let blob_id = attachement.id.clone();
                        let params = attachement.params.as_object().unwrap();
                        let file_name = params
                            .get("fileName")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&attachement.id);
                        sender.send(CommunicationEvent::Attachment(ReceivedAttachment {
                            card_id: patch.card_id.clone(),
                            message_id: patch.message_id.clone(),
                            file_name: file_name.to_string(),
                            // http://huly.local:4030/blob/:workspace/:blobId/:filename
                            url: context
                                .server_config
                                .files_url
                                .clone()
                                .replace(":workspace", &workspace.to_string())
                                .replace(":blobId", &blob_id)
                                .replace(
                                    ":filename",
                                    &percent_encoding::percent_encode(
                                        file_name.as_bytes(),
                                        NON_ALPHANUMERIC,
                                    )
                                    .to_string(),
                                ),
                        }))?;
                    }
                }
            }
            CommunicationDomainEventKind::ReactionPatch(patch) => {
                if tracked_message_ids.contains(&patch.message_id) {
                    let person_info = get_person_info(&mut context, &patch.social_id).await?;
                    if patch.operation.opcode == "add" {
                        sender.send(CommunicationEvent::Reaction(ReceivedReaction {
                            card_id: patch.card_id.clone(),
                            message_id: patch.message_id.clone(),
                            person: person_info.to_string(),
                            reaction: patch.operation.reaction,
                        }))?;
                    }
                }
            }
            CommunicationDomainEventKind::ThreadPatch(patch) => {
                if let ThreadPatchOperation::Attach(op) = patch.operation
                    && tracked_message_ids.contains(&patch.message_id)
                {
                    follow_card_ids.insert(op.thread_id.clone(), MAX_FOLLOW_MESSAGES);
                }
            }
            _ => continue,
        }
    }
}
