use anyhow::Result;

use async_trait::async_trait;
use hulyrs::services::{
    card::{self, Card, CardSpace},
    collaborator::CollaborativeDoc,
    event::Class,
    transactor::{
        document::{CreateDocumentBuilder, DocumentClient, FindOptions},
        utils::generate_object_id,
    },
};
use serde::Deserialize;
use serde_json::{from_value, json};

use crate::{context::AgentContext, tools::ToolImpl, types::ToolResultContent};

pub struct CreateCardTool {
    pub description: serde_json::Value,
}

fn default_space() -> String {
    "card:space:Default".to_string()
}

fn default_type() -> String {
    "card:types:Document".to_string()
}

#[derive(Deserialize)]
struct CreateCardToolArgs {
    title: String,
    #[serde(default = "default_space")]
    space: String,
    #[serde(default = "default_type")]
    card_type: String,
}

pub struct ReadCardTool {
    pub description: serde_json::Value,
}

#[derive(Deserialize)]
struct ReadCardToolArgs {
    id: String,
}

pub struct UpdateCardTool {
    pub description: serde_json::Value,
}

#[derive(Deserialize)]
struct UpdateCardToolArgs {
    id: String,
    content: String,
}

pub struct GetCardSpacesTool {
    pub description: serde_json::Value,
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
            .object_class(args.card_type.clone())
            .created_by(social_id.clone())
            .modified_by(social_id)
            .created_on(chrono::Utc::now())
            .modified_on(chrono::Utc::now())
            .object_space(args.space.clone())
            .attributes(json!({
                "title": args.title,
            }))
            .build()?;

        _ = context.tx_client.tx::<_, serde_json::Value>(event).await?;

        let card = json!({
            "id": card_id,
            "link": format!("huly://card/{}", card_id),
            "title": args.title,
            "type": args.card_type,
            "space": args.space,
        });

        Ok(vec![ToolResultContent::text(format!(
            "Successfully created a card: {}",
            card,
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
        tracing::debug!(card_id = args.id, "Read card");

        let doc = CollaborativeDoc {
            object_id: args.id,
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
        tracing::debug!(card_id = args.id, "Update card");

        let doc = CollaborativeDoc {
            object_id: args.id.clone(),
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
            args.id,
        ))])
    }
}

#[async_trait]
impl ToolImpl for GetCardSpacesTool {
    fn desciption(&self) -> &serde_json::Value {
        &self.description
    }

    async fn call(
        &mut self,
        context: &AgentContext,
        _: serde_json::Value,
    ) -> Result<Vec<ToolResultContent>> {
        tracing::debug!("Get all card spaces");

        let spaces: Vec<CardSpace> = context
            .tx_client
            .find_all(card::CardSpace::CLASS, json!({}), &FindOptions::default())
            .await?
            .value
            .into_iter()
            .filter_map(|v| from_value(v).ok())
            .collect();

        Ok(vec![ToolResultContent::text(format!(
            "Card spaces found {:?}",
            spaces
        ))])
    }
}
