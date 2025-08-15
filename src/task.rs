use std::{collections::HashMap, fmt::Display};

use anyhow::Result;
use indexmap::IndexMap;
use itertools::Itertools;
use tokio::sync::mpsc;

use crate::{huly::streaming::types::CommunicationEvent, types::Message};

pub const MAX_FOLLOW_MESSAGES: u8 = 10;

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

pub async fn task_multiplexer(
    mut receiver: mpsc::UnboundedReceiver<CommunicationEvent>,
    sender: mpsc::UnboundedSender<Task>,
    social_id: String,
) -> Result<()> {
    tracing::debug!("Start task multiplexer");
    let mut channel_messages = HashMap::<String, IndexMap<String, ChannelMessage>>::new();

    while let Some(event) = receiver.recv().await {
        tracing::debug!("Received event: {:?}", event);
        match event {
            CommunicationEvent::ReceviedReaction(reaction) => {
                if let Some(messages) = channel_messages.get_mut(&reaction.channel_id) {
                    if let Some(message) = messages.get_mut(&reaction.message_id) {
                        message.reactions.push(Reaction {
                            person: reaction.person,
                            reaction: reaction.reaction,
                        });
                    }
                }
                continue;
            }
            CommunicationEvent::ReceviedAttachment(attachement) => {
                if let Some(messages) = channel_messages.get_mut(&attachement.channel_id) {
                    if let Some(message) = messages.get_mut(&attachement.message_id) {
                        message.attachments.push(Attachment {
                            file_name: attachement.file_name,
                            url: attachement.url,
                        });
                    }
                }
                continue;
            }
            _ => {}
        }

        let CommunicationEvent::ReceivedMessage(new_message) = event else {
            continue;
        };
        channel_messages
            .entry(new_message.card_id.clone())
            .or_default()
            .insert(
                new_message.message_id.clone(),
                ChannelMessage {
                    message_id: new_message.message_id,
                    person_info: new_message.person_info.to_string(),
                    date: new_message.date,
                    content: new_message.content,
                    attachments: vec![],
                    reactions: vec![],
                },
            );

        // skip messages from the same social_id for follow mode
        if !new_message.is_mention && new_message.social_id == social_id {
            continue;
        }

        let task = Task {
            id: 0,
            kind: TaskKind::FollowChat {
                channel_id: new_message.card_id.clone(),
                channel_title: new_message.card_title.unwrap_or_default(),
                content: format_messages(
                    channel_messages.get(&new_message.card_id).unwrap().values(),
                ),
            },
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        sender.send(task)?;
    }
    tracing::debug!("Task multiplexer terminated");
    Ok(())
}
