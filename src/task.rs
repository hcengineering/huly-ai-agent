use serde::{Deserialize, Serialize};

use crate::types::Message;

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

impl Task {
    pub fn direct_question(user: &str, content: &str) -> Self {
        Self {
            kind: TaskKind::DirectQuestion,
            content: format!("|user_question {user}|{content}"),
            priority: 0,
            messages: vec![Message::user(&format!("|user_question {user}|{content}"))],
        }
    }
}
