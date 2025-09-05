// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use base64::Engine;
use hulyrs::services::{
    core::WorkspaceUuid,
    transactor::{
        comm::{
            BlobData, BlobPatchEventBuilder, BlobPatchOperation, CreateMessageEvent,
            CreateMessageEventBuilder, Envelope, MessageRequestType, MessageType,
        },
        kafka::KafkaProducer,
    },
};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    huly::blob::BlobClient,
    types::{
        AssistantContent, ContentFormat, Image, Message, Text, ToolCall, ToolFunction, ToolResult,
        ToolResultContent, UserContent,
    },
    utils::{escape_markdown, safe_truncated},
};

fn format_tool_function(function: &ToolFunction) -> String {
    let name = match function.name.as_str() {
        "send_message" => "✉️",
        "create_entities"
        | "create_relations"
        | "add_observations"
        | "delete_entities"
        | "delete_observations"
        | "delete_relations"
        | "read_graph"
        | "search_nodes"
        | "open_nodes" => &format!("🧠 {}", function.name),
        _ => &function.name,
    };

    format!(
        "{name}: \n```json\n{}\n```\n",
        &serde_json::to_string_pretty(&function.arguments)
            .unwrap_or(function.arguments.to_string())
    )
}

pub struct HulyChannelLogWriter {
    sender: mpsc::UnboundedSender<(CreateMessageEvent, Vec<Image>)>,
    social_id: String,
    channel_id: String,
}

impl HulyChannelLogWriter {
    pub fn new(
        sender: mpsc::UnboundedSender<(CreateMessageEvent, Vec<Image>)>,
        social_id: String,
        channel_id: String,
    ) -> Self {
        Self {
            sender,
            social_id,
            channel_id,
        }
    }

    fn send_message(&self, msg: &str, attachements: Vec<Image>) {
        if self.sender.is_closed() {
            eprintln!("Channel log worker is closed");
            return;
        }
        let message_id = Uuid::new_v4().as_u64_pair().0.to_string();
        if let Ok(create_event) = CreateMessageEventBuilder::default()
            .message_type(MessageType::Message)
            .card_id(&self.channel_id)
            .message_id(message_id)
            .card_type("chat:masterTag:Thread")
            .content(msg)
            .social_id(&self.social_id)
            .build()
        {
            let _ = self.sender.send((create_event, attachements));
        }
    }

    pub fn trace_log(&self, msg: &str) {
        self.send_message(msg, vec![]);
    }

    pub fn trace_message(&self, message: &Message) {
        match message {
            Message::User { content } => {
                let mut msg = String::new();
                let mut attachements = vec![];
                match content.first().unwrap() {
                    UserContent::Text(Text { text }) => msg.push_str(&escape_markdown(text)),
                    UserContent::ToolResult(ToolResult { content, .. }) => {
                        content.iter().for_each(|c| match c {
                            ToolResultContent::Text(Text { text }) => msg.push_str(&format!(
                                "⚙️ \n```\n{}\n```\n",
                                safe_truncated(text, 512)
                            )),
                            ToolResultContent::Image(img) => {
                                if let Some(ContentFormat::String) = img.format {
                                    msg.push_str(format!("\n[image]({})", img.data).as_str());
                                } else {
                                    attachements.push(img.clone())
                                }
                            }
                        });
                    }
                    UserContent::Image(img) => attachements.push(img.clone()),
                    _ => msg.push_str("unknown"),
                };

                self.send_message(&format!("👨‍: {}", &msg), attachements);
            }
            Message::Assistant { content } => {
                let msg = content
                    .iter()
                    .map(|c| match c {
                        AssistantContent::Text(Text { text }) => escape_markdown(text),
                        AssistantContent::ToolCall(ToolCall { function, .. }) => {
                            format!("⚙️ {}", format_tool_function(function))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                self.send_message(&format!("🤖: {msg}"), vec![]);
            }
        }
    }
}

pub async fn run_channel_log_worker(
    event_publisher: KafkaProducer,
    blob_client: BlobClient,
    workspace: WorkspaceUuid,
    mut receiver: mpsc::UnboundedReceiver<(CreateMessageEvent, Vec<Image>)>,
) {
    while let Some((event, attachements)) = receiver.recv().await {
        let card_id = event.card_id.clone();
        let social_id = event.social_id.clone();
        let message_id = event.message_id.clone().unwrap();
        let create_event = Envelope::new(MessageRequestType::CreateMessage, event);
        let _ = event_publisher
            .tx(workspace, create_event, Some(&card_id))
            .await;
        for image in attachements {
            if image
                .format
                .is_none_or(|f| matches!(f, ContentFormat::Base64))
            {
                let blob_id = Uuid::new_v4().to_string();
                let content = base64::engine::general_purpose::STANDARD
                    .decode(image.data)
                    .unwrap();
                let size = content.len() as u32;
                let mime_type = image
                    .media_type
                    .unwrap_or(crate::types::ImageMediaType::PNG)
                    .to_mime_type();
                if blob_client
                    .upload_file(&blob_id, mime_type, content)
                    .await
                    .is_ok()
                {
                    let attachement_event = BlobPatchEventBuilder::default()
                        .card_id(&card_id)
                        .message_id(&message_id)
                        .operations(vec![BlobPatchOperation::Attach {
                            blobs: vec![BlobData {
                                blob_id: blob_id.clone(),
                                mime_type: mime_type.to_string(),
                                file_name: format!("{blob_id}.png"),
                                size,
                                metadata: None,
                            }],
                        }])
                        .social_id(&social_id)
                        .build()
                        .unwrap();

                    let add_attachement =
                        Envelope::new(MessageRequestType::BlobPatch, attachement_event);
                    let _ = event_publisher
                        .tx(workspace, add_attachement, Some(&card_id))
                        .await;
                }
            }
        }
    }
}
