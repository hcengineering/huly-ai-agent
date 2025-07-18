use std::{collections::HashMap, fmt::Display};

use anyhow::Result;
use tokio::sync::mpsc;

use crate::{huly::streaming::types::ReceivedMessage, types::Message};

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
            } => Message::user(&format!(
                "|direct|user:[{name}]({person_id})|message:{content}"
            )),
            TaskKind::Mention {
                person_id,
                name,
                channel_id,
                channel_title,
                content,
                ..
            } => Message::user(&format!(
                "|user_mention|user:[{name}]({person_id})|channel:[{channel_title}]({channel_id})|message:{content}"
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

pub async fn task_multiplexer(
    mut receiver: mpsc::UnboundedReceiver<(ReceivedMessage, bool)>,
    sender: mpsc::UnboundedSender<Task>,
    social_id: String,
) -> Result<()> {
    tracing::debug!("Start task multiplexer");
    let mut channel_messages = HashMap::<String, Vec<String>>::new();

    while let Some((new_message, is_mention)) = receiver.recv().await {
        tracing::debug!("Received message: {:?}", new_message);
        let message_text = format!(
            "[{}]({}) _{}_:\n{}",
            new_message.person_name.clone().unwrap_or_default(),
            new_message.person_id.clone().unwrap_or_default(),
            new_message.date,
            new_message.content
        );
        channel_messages
            .entry(new_message.card_id.clone())
            .and_modify(|v| v.push(message_text.clone()))
            .or_insert(vec![message_text.clone()]);

        // skip messages from the same social_id for follow mode
        if !is_mention && new_message.social_id == social_id {
            continue;
        }

        let task = Task {
            id: 0,
            kind: if is_mention {
                TaskKind::Mention {
                    person_id: new_message.person_id.unwrap_or_default(),
                    social_id: new_message.social_id,
                    name: new_message.person_name.unwrap_or_default(),
                    channel_id: new_message.card_id,
                    channel_title: new_message.card_title.unwrap_or_default(),
                    content: new_message.content,
                }
            } else {
                TaskKind::FollowChat {
                    channel_id: new_message.card_id.clone(),
                    channel_title: new_message.card_title.unwrap_or_default(),
                    content: channel_messages
                        .get(&new_message.card_id)
                        .unwrap()
                        .join("\n\n"),
                }
            },
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        sender.send(task)?;
    }
    tracing::debug!("Task multiplexer terminated");
    Ok(())
}
