use anyhow::Result;
use tokio::sync::mpsc;

use crate::{huly::streaming::types::CreateMessage, types::Message};

#[derive(Debug, Clone)]
pub struct Task {
    pub id: i64,
    pub kind: TaskKind,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub enum TaskKind {
    DirectQuestion {
        social_id: String,
        content: String,
    },
    Mention {
        social_id: String,
        channel_id: String,
        content: String,
    },
    Research,
    Sleep,
}

impl From<TaskKind> for Message {
    fn from(kind: TaskKind) -> Self {
        match kind {
            TaskKind::DirectQuestion { social_id, content } => {
                Self::user(&format!("|direct|user:{}|message:{}", social_id, content))
            }
            TaskKind::Mention {
                social_id,
                channel_id,
                content,
            } => Self::user(&format!(
                "|user_mention|user:{}|channel:{}|message:{}",
                social_id, channel_id, content
            )),
            TaskKind::Research => Self::user("|research|"),
            TaskKind::Sleep => Self::user("|sleep|"),
        }
    }
}

pub async fn task_multiplexer(
    mut receiver: mpsc::UnboundedReceiver<CreateMessage>,
    sender: mpsc::UnboundedSender<Task>,
) -> Result<()> {
    tracing::debug!("Start task multiplexer");

    while let Some(new_message) = receiver.recv().await {
        tracing::debug!("Received message: {:?}", new_message);
        let task = Task {
            id: 0,
            kind: TaskKind::Mention {
                social_id: new_message.social_id,
                channel_id: new_message.card_id,
                content: new_message.content,
            },
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        sender.send(task)?;
    }
    tracing::debug!("Task multiplexer terminated");
    Ok(())
}
