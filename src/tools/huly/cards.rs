use anyhow::Result;

use async_trait::async_trait;
use hulyrs::services::{
    card::Card,
    collaborator::CollaborativeDoc,
    event::Class,
    transactor::{document::CreateDocumentBuilder, utils::generate_object_id},
};
use serde::Deserialize;
use serde_json::json;

use crate::{context::AgentContext, tools::ToolImpl, types::ToolResultContent};

pub struct CreateCardTool {
    pub description: serde_json::Value,
}

#[derive(Deserialize)]
struct CreateCardToolArgs {
    title: String,
}

pub struct ReadCardTool {
    pub description: serde_json::Value,
}

#[derive(Deserialize)]
struct ReadCardToolArgs {
    card_id: String,
}

pub struct UpdateCardTool {
    pub description: serde_json::Value,
}

#[derive(Deserialize)]
struct UpdateCardToolArgs {
    card_id: String,
    content: String,
}

#[async_trait]
impl ToolImpl for CreateCardTool {
    fn desciption(&self) -> &serde_json::Value {
        &self.description
    }

    async fn call(
        &mut self,
        context: &AgentContext,
        args: serde_json::Value,
    ) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<CreateCardToolArgs>(args)?;
        tracing::debug!(title = args.title, "Create card");

        let social_id = context.account_info.social_id.clone();
        let card_id = generate_object_id();

        let event = CreateDocumentBuilder::default()
            .object_id(card_id.clone())
            .object_class("card:class:Card")
            .created_by(social_id.clone())
            .modified_by(social_id)
            .created_on(chrono::Utc::now())
            .modified_on(chrono::Utc::now())
            .object_space("card:space:Default")
            .attributes(json!({
                "title": args.title,
            }))
            .build()?;

        _ = context.tx_client.tx::<_, serde_json::Value>(event).await?;

        Ok(vec![ToolResultContent::text(format!(
            "Successfully created a card with title {} and id {}",
            args.title, card_id
        ))])
    }
}

#[async_trait]
impl ToolImpl for ReadCardTool {
    fn desciption(&self) -> &serde_json::Value {
        &self.description
    }

    async fn call(
        &mut self,
        context: &AgentContext,
        args: serde_json::Value,
    ) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<ReadCardToolArgs>(args)?;
        tracing::debug!(card_id = args.card_id, "Read card");

        let doc = CollaborativeDoc {
            object_id: args.card_id,
            object_class: Card::CLASS.to_string(),
            object_attr: "content".to_string(),
        };

        let markup = context.collaborator_client.get_markup(&doc, None).await?;
        let markup = serde_json::from_str::<hulyrs::text::MarkupNode>(&markup)?;
        let content = hulyrs::text::markup_to_markdown(&markup, "".to_string(), "".to_string());
        Ok(vec![ToolResultContent::text(format!(
            "Successfully read a card {}",
            content,
        ))])
    }
}

#[async_trait]
impl ToolImpl for UpdateCardTool {
    fn desciption(&self) -> &serde_json::Value {
        &self.description
    }

    async fn call(
        &mut self,
        context: &AgentContext,
        args: serde_json::Value,
    ) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<UpdateCardToolArgs>(args)?;
        tracing::debug!(card_id = args.card_id, "Update card");

        let doc = CollaborativeDoc {
            object_id: args.card_id.clone(),
            object_class: Card::CLASS.to_string(),
            object_attr: "content".to_string(),
        };

        let markup = hulyrs::text::markdown_to_markup(&args.content);
        let markup = serde_json::to_string(&markup)?;

        context
            .collaborator_client
            .update_markup(&doc, markup)
            .await?;

        Ok(vec![ToolResultContent::text(format!(
            "Successfully updated a card {}",
            args.card_id,
        ))])
    }
}
