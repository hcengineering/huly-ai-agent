// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use hulyrs::services::{
    transactor::{
        comm::{
            CreateMessageEvent, CreateMessageEventBuilder, Envelope, MessageRequestType,
            MessageType,
        },
        kafka::KafkaProducer,
    },
    types::WorkspaceUuid,
};
use tokio::sync::mpsc;
use tracing_subscriber::Layer;

struct VisitFmt<'a>(&'a mut String);

impl<'a> tracing::field::Visit for VisitFmt<'a> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0.push_str(&format!("{value:?}"));
        }
    }
}

pub struct HulyChannelLogWriter {
    level: tracing::level_filters::LevelFilter,
    sender: mpsc::UnboundedSender<CreateMessageEvent>,
    social_id: String,
    channel_id: String,
}

impl HulyChannelLogWriter {
    pub fn new(
        level: tracing::Level,
        sender: mpsc::UnboundedSender<CreateMessageEvent>,
        social_id: String,
        channel_id: String,
    ) -> Self {
        Self {
            level: level.into(),
            sender,
            social_id,
            channel_id,
        }
    }
}

impl<S> Layer<S> for HulyChannelLogWriter
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        if !self.level.enabled(event.metadata(), ctx)
            || !event.metadata().target().starts_with("huly_ai_agent")
            || event.metadata().fields().field("log_message").is_none()
        {
            return;
        }
        if self.sender.is_closed() {
            eprintln!("Channel log worker is closed");
            return;
        }
        let mut message = String::new();
        event.record(&mut VisitFmt(&mut message));

        if let Ok(create_event) = CreateMessageEventBuilder::default()
            .message_type(MessageType::Message)
            .card_id(&self.channel_id)
            .card_type("chat:masterTag:Channel")
            .content(message)
            .social_id(&self.social_id)
            .build()
        {
            let _ = self.sender.send(create_event);
        }
    }
}

pub async fn run_channel_log_worker(
    event_publisher: KafkaProducer,
    workspace: WorkspaceUuid,
    mut receiver: mpsc::UnboundedReceiver<CreateMessageEvent>,
) {
    while let Some(event) = receiver.recv().await {
        let card_id = event.card_id.clone();
        let create_event = Envelope::new(MessageRequestType::CreateMessage, event);
        let _ = event_publisher
            .tx(workspace, create_event, Some(&card_id))
            .await;
    }
}
