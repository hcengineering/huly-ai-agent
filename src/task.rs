use std::{
    collections::HashMap,
    fmt::Display,
    time::{Duration, Instant},
};

use anyhow::Result;
use indexmap::IndexMap;
use itertools::Itertools;
use tokio::{select, sync::mpsc};

use crate::{
    huly::streaming::types::{CommunicationEvent, ReceivedMessage},
    types::Message,
};

pub const MAX_FOLLOW_MESSAGES: u8 = 10;
pub const TASK_START_DELAY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct Task {
    pub id: i64,
    pub kind: TaskKind,
    #[allow(unused)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[allow(unused)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

pub struct Attachment {
    pub file_name: String,
    pub url: String,
}

pub struct Reaction {
    pub person: String,
    pub reaction: String,
}

pub struct ChannelMessage {
    pub message_id: String,
    pub person_info: String,
    pub date: String,
    pub content: String,
    pub attachments: Vec<Attachment>,
    pub reactions: Vec<Reaction>,
}

#[derive(Debug, Clone)]
pub enum TaskKind {
    DirectQuestion {
        person_id: String,
        social_id: String,
        name: String,
        content: String,
    },
    Mention {
        person_id: String,
        social_id: String,
        name: String,
        channel_id: String,
        channel_title: String,
        message_id: String,
        content: String,
    },
    FollowChat {
        channel_id: String,
        channel_title: String,
        content: String,
    },
    Research,
    Sleep,
}

impl Display for TaskKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            TaskKind::DirectQuestion { .. } => "direct_question",
            TaskKind::Mention { .. } => "mention",
            TaskKind::FollowChat { .. } => "follow_chat",
            TaskKind::Research => "research",
            TaskKind::Sleep => "sleep",
        };
        f.write_str(name)
    }
}

impl TaskKind {
    pub fn to_message(&self) -> Message {
        match self {
            TaskKind::DirectQuestion {
                person_id,
                name,
                content,
                ..
                // TODO: add message_id
            } => Message::user(&format!(
                "|direct|user:[{name}]({person_id})|message_id:unknown|message:{content}"
            )),
            TaskKind::Mention {
                person_id,
                name,
                channel_id,
                channel_title,
                content,
                message_id,
                ..
            } => Message::user(&format!(
                "|user_mention|user:[{name}]({person_id})|channel:[{channel_title}]({channel_id})|message_id:{message_id}|message:{content}"
            )),
            TaskKind::FollowChat {
                channel_id,
                channel_title,
                content,
            } => Message::user(&format!(
                "|follow_chat|channel:[{channel_title}]({channel_id})|chat_log:{content}"
            )),
            TaskKind::Research => Message::user("|research|"),
            TaskKind::Sleep => Message::user("|sleep|"),
        }
    }
}

fn format_messages<'a>(messages: impl IntoIterator<Item = &'a ChannelMessage>) -> String {
    messages
        .into_iter()
        .map(|m| {
            let attachements_block = if m.attachments.is_empty() {
                "".to_string()
            } else {
                format!(
                    "\n- attachments\n{}",
                    m.attachments
                        .iter()
                        .map(|a| format!("  - [{}]({})", a.file_name, a.url))
                        .join("\n")
                )
            };
            let reactions_block = if m.reactions.is_empty() {
                "".to_string()
            } else {
                format!(
                    "\n- reactions\n{}",
                    m.reactions
                        .iter()
                        .map(|r| format!("  - {}|{}", r.person, r.reaction))
                        .join("\n")
                )
            };

            format!(
                "{}|{} _{}_:\n{}{}{}",
                m.message_id, m.person_info, m.date, m.content, attachements_block, reactions_block
            )
        })
        .join("\n\n")
}

async fn process_incoming_event(
    receiver: &mut mpsc::UnboundedReceiver<CommunicationEvent>,
    channel_messages: &mut HashMap<String, IndexMap<String, ChannelMessage>>,
    social_id: &str,
) -> Result<(bool, Option<ReceivedMessage>)> {
    let Some(event) = receiver.recv().await else {
        return Ok((false, None));
    };
    tracing::debug!("Received event: {:?}", event);
    match event {
        CommunicationEvent::ReceivedReaction(reaction) => {
            if let Some(messages) = channel_messages.get_mut(&reaction.channel_id) {
                if let Some(message) = messages.get_mut(&reaction.message_id) {
                    message.reactions.push(Reaction {
                        person: reaction.person,
                        reaction: reaction.reaction,
                    });
                }
            }
            return Ok((true, None));
        }
        CommunicationEvent::ReceivedAttachment(attachement) => {
            if let Some(messages) = channel_messages.get_mut(&attachement.channel_id) {
                if let Some(message) = messages.get_mut(&attachement.message_id) {
                    message.attachments.push(Attachment {
                        file_name: attachement.file_name,
                        url: attachement.url,
                    });
                }
            }
            return Ok((true, None));
        }
        _ => {}
    }

    let CommunicationEvent::ReceivedMessage(new_message) = event else {
        return Ok((true, None));
    };
    channel_messages
        .entry(new_message.card_id.clone())
        .or_default()
        .insert(
            new_message.message_id.clone(),
            ChannelMessage {
                message_id: new_message.message_id.clone(),
                person_info: new_message.person_info.to_string(),
                date: new_message.date.clone(),
                content: new_message.content.clone(),
                attachments: vec![],
                reactions: vec![],
            },
        );

    // skip messages from the same social_id for follow mode
    if !new_message.is_mention && new_message.social_id == social_id {
        return Ok((true, None));
    }
    Ok((true, Some(new_message)))
}

pub async fn task_multiplexer(
    mut receiver: mpsc::UnboundedReceiver<CommunicationEvent>,
    sender: mpsc::UnboundedSender<Task>,
    social_id: String,
) -> Result<()> {
    tracing::debug!("Start task multiplexer");
    let mut channel_messages = HashMap::<String, IndexMap<String, ChannelMessage>>::new();
    let mut new_messages = IndexMap::<String, (ReceivedMessage, Instant)>::new();

    let mut delay = Duration::from_secs(u64::MAX);
    loop {
        select! {
            res = process_incoming_event(&mut receiver, &mut channel_messages, &social_id) => {
                match res {
                    Ok((true, new_message)) => {
                        if let Some(new_message) = new_message {
                            new_messages.insert(new_message.card_id.clone(), (new_message, Instant::now().checked_add(TASK_START_DELAY).unwrap()));
                        }
                        // recalculate delay
                        delay = Duration::from_secs(u64::MAX);
                        for (_, (_, time)) in new_messages.iter() {
                            delay = delay.min(time.duration_since(Instant::now()));
                        }
                    },
                    Ok((false, _)) => break,
                    Err(e) => {
                        tracing::error!("Error processing incoming message: {e}");
                        break;
                    }
                }
            },
            _ = tokio::time::sleep(delay) => {
                new_messages.retain(|_, (message, time)| if *time > Instant::now() {
                    true
                } else {
                    sender.send(Task {
                        id: 0,
                        kind: TaskKind::FollowChat {
                            channel_id: message.card_id.clone(),
                            channel_title: message.card_title.clone().unwrap_or_default(),
                            content: format_messages(
                                channel_messages.get(&message.card_id).unwrap().values(),
                            ),
                        },
                        created_at: chrono::Utc::now(),
                        updated_at: chrono::Utc::now(),
                    }).unwrap();
                    false
                });
                // recalculate delay
                delay = Duration::from_secs(u64::MAX);
                for (_, (_, time)) in new_messages.iter() {
                    delay = delay.min(time.duration_since(Instant::now()));
                }
            },
        }
    }
    Ok(())
}
