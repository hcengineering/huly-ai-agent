use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::{huly::streaming::types::CreateMessage, types::Message};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub kind: TaskKind,
    pub content: String,
    pub priority: i32,
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskKind {
    DirectQuestion,
    Mention,
    Research,
    Sleep,
}

pub async fn task_multiplexer(
    mut receiver: mpsc::UnboundedReceiver<CreateMessage>,
    sender: mpsc::UnboundedSender<Task>,
) -> Result<()> {
    tracing::debug!("Start task multiplexer");

    while let Some(new_message) = receiver.recv().await {
        tracing::debug!("Received message: {:?}", new_message);
        let txt = format!(
            "|user_mention|user:{}|channel:{}|message:{}",
            new_message.social_id, new_message.card_id, new_message.content
        );
        let message = Message::user(&txt);
        let task = Task {
            kind: TaskKind::Mention,
            content: txt,
            priority: 0,
            messages: vec![message],
        };
        sender.send(task)?;
    }
    tracing::debug!("Task multiplexer terminated");
    Ok(())
}
