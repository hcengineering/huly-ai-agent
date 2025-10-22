// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::fmt::Display;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum CommunicationEvent {
    Message(ReceivedMessage),
    Reaction(ReceivedReaction),
    Attachment(ReceivedAttachment),
}

#[derive(Debug, Deserialize)]
pub struct ReceivedMessage {
    pub card_id: String,
    pub card_title: Option<String>,
    pub content: String,
    pub social_id: String,
    pub person_info: PersonInfo,
    pub message_id: String,
    pub date: String,
}

#[derive(Debug, Deserialize)]
pub struct ReceivedAttachment {
    pub card_id: String,
    pub message_id: String,
    pub file_name: String,
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct ReceivedReaction {
    pub card_id: String,
    pub message_id: String,
    pub person: String,
    pub reaction: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PersonInfo {
    pub person_id: String,
    pub person_name: String,
}

impl Display for PersonInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}]({})", self.person_id, self.person_name)
    }
}
