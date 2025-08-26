// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{collections::HashMap, path::PathBuf};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use base64::Engine;
use hulyrs::services::transactor::{
    TransactorClient,
    backend::http::HttpBackend,
    comm::{
        BlobData, BlobPatchEventBuilder, BlobPatchOperation, CreateMessageEventBuilder, Envelope,
        MessageRequestType, MessageType,
    },
};
use reqwest::header::{self, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::Value;
use tokio::{fs::File, io::AsyncReadExt};
use uuid::Uuid;

use crate::{
    config::Config,
    context::AgentContext,
    huly::{self, blob::BlobClient},
    state::AgentState,
    tools::{ToolImpl, ToolSet, files::normalize_path},
    types::{ContentFormat, Image, ImageMediaType, Text, ToolResultContent},
};

pub struct HulyToolSet {
    presenter: Option<HulyAiPresenterClient>,
    tools: Vec<serde_json::Value>,
    presenter_tools: Vec<String>,
}

impl ToolSet for HulyToolSet {
    fn get_tools<'a>(
        &self,
        config: &'a Config,
        context: &'a AgentContext,
        _state: &'a AgentState,
    ) -> Vec<Box<dyn ToolImpl>> {
        let mut tools: Vec<Box<dyn ToolImpl>> = vec![
            Box::new(SendMessageTool {
                social_id: context.social_id.clone(),
                tx_client: context.tx_client.clone(),
            }),
            Box::new(AddMessageReactionTool {
                social_id: context.social_id.clone(),
                tx_client: context.tx_client.clone(),
            }),
            Box::new(AddMessageAttachementTool {
                workspace: config.workspace.clone(),
                social_id: context.social_id.clone(),
                tx_client: context.tx_client.clone(),
                blob_client: context.blob_client.clone(),
            }),
            Box::new(SendMessageTool {
                social_id: context.social_id.clone(),
                tx_client: context.tx_client.clone(),
            }),
        ];
        if let Some(presenter) = self.presenter.as_ref() {
            for tool in &self.presenter_tools {
                tools.push(Box::new(HulyPresenterTool {
                    client: presenter.clone(),
                    method: tool.clone(),
                }));
            }
        }
        tools
    }

    fn get_tool_descriptions(&self, _config: &Config) -> Vec<serde_json::Value> {
        self.tools.clone()
    }

    fn get_system_prompt(&self, _config: &Config) -> String {
        include_str!("system_prompt.txt").to_string()
    }
}

pub async fn create_huly_tool_set(config: &Config, context: &AgentContext) -> Result<HulyToolSet> {
    let (presenter, params) = if let Some(url) = &config.huly.presenter_url {
        let presenter = create_presenter_client(url.clone(), context.token.clone()).await?;
        let params = presenter.get_params_schema().await;
        (Some(presenter), params)
    } else {
        (None, Ok(HashMap::new()))
    };
    let mut tools: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("tools.json")).unwrap();
    let mut presenter_tools = Vec::new();
    if let Ok(params) = params {
        for tool in &mut tools {
            let tool_obj = tool.as_object_mut().unwrap();
            let tool_name = tool_obj
                .get("function")
                .unwrap()
                .get("name")
                .unwrap()
                .as_str()
                .unwrap();
            let Some(params) = params.get(tool_name.trim_start_matches("huly_")) else {
                continue;
            };
            presenter_tools.push(tool_name.to_string());
            tool_obj.insert("parameters".to_string(), params.clone());
        }
    }

    Ok(HulyToolSet {
        presenter,
        tools,
        presenter_tools,
    })
}

struct SendMessageTool {
    social_id: String,
    tx_client: TransactorClient<HttpBackend>,
}

#[derive(Deserialize)]
struct SendMessageToolArgs {
    channel: String,
    content: String,
}

struct AddMessageReactionTool {
    social_id: String,
    tx_client: TransactorClient<HttpBackend>,
}

#[derive(Deserialize)]
struct AddMessageReactionToolArgs {
    channel: String,
    message_id: String,
    reaction: String,
}

struct AddMessageAttachementTool {
    workspace: PathBuf,
    social_id: String,
    tx_client: TransactorClient<HttpBackend>,
    blob_client: BlobClient,
}

#[derive(Deserialize)]
struct AddMessageAttachementToolArgs {
    channel: String,
    message_id: String,
    attachement_name: String,
    attachement_data: String,
}

#[async_trait]
impl ToolImpl for SendMessageTool {
    fn name(&self) -> &str {
        "huly_send_message"
    }

    async fn call(&mut self, args: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<SendMessageToolArgs>(args)?;
        tracing::debug!(
            channel = args.channel,
            content = args.content,
            "Send message to channel"
        );
        let card_id = args.channel;

        let create_event = CreateMessageEventBuilder::default()
            .message_type(MessageType::Message)
            .card_id(card_id)
            .card_type("chat:masterTag:Channel")
            .content(args.content)
            .social_id(&self.social_id)
            .build()
            .unwrap();

        let create_event = Envelope::new(MessageRequestType::CreateMessage, create_event);

        let res = self.tx_client.tx::<_, Value>(create_event).await?;
        Ok(vec![ToolResultContent::text(format!(
            "Message sent, message_id is {}",
            res["messageId"]
        ))])
    }
}

#[async_trait]
impl ToolImpl for AddMessageReactionTool {
    fn name(&self) -> &str {
        "huly_add_message_reaction"
    }

    async fn call(&mut self, args: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<AddMessageReactionToolArgs>(args)?;
        tracing::debug!(
            channel = args.channel,
            message_id = args.message_id,
            reaction = args.reaction,
            "Add message reaction"
        );
        huly::add_reaction(
            &self.tx_client,
            &args.channel,
            &args.message_id,
            &self.social_id,
            &args.reaction,
        )
        .await?;
        Ok(vec![ToolResultContent::text(format!(
            "Successfully added reaction to message with message_id {}",
            args.message_id
        ))])
    }
}

#[async_trait]
impl ToolImpl for AddMessageAttachementTool {
    fn name(&self) -> &str {
        "add_message_attachement"
    }

    async fn call(&mut self, args: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<AddMessageAttachementToolArgs>(args)?;
        tracing::debug!(
            channel = args.channel,
            message_id = args.message_id,
            attachement_name = args.attachement_name,
            "Add message attachement"
        );

        let (mime_type, content) = if args.attachement_data.starts_with("data:") {
            // data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAADIA...
            let data = args.attachement_data.split(',').collect::<Vec<&str>>();
            let mime_type = data[0][5..].split(';').collect::<Vec<&str>>()[0];
            let content = base64::engine::general_purpose::STANDARD.decode(data[1])?;
            (mime_type.to_string(), content)
        } else {
            let path = normalize_path(&self.workspace, &args.attachement_data);
            let mut file = File::open(path).await?;
            let mut content = Vec::new();
            let mime_type = mime_guess::from_path(args.attachement_data)
                .first_or_text_plain()
                .to_string();
            file.read_to_end(&mut content).await?;
            (mime_type, content)
        };

        let size = content.len() as u32;
        let blob_id = Uuid::new_v4().to_string();
        self.blob_client
            .upload_file(&blob_id, &mime_type, content)
            .await?;

        let attachement_event = BlobPatchEventBuilder::default()
            .card_id(args.channel)
            .message_id(&args.message_id)
            .operations(vec![BlobPatchOperation::Attach {
                blobs: vec![BlobData {
                    blob_id,
                    mime_type,
                    file_name: args.attachement_name,
                    size,
                    metadata: None,
                }],
            }])
            .social_id(&self.social_id)
            .build()
            .unwrap();

        let add_attachement = Envelope::new(MessageRequestType::BlobPatch, attachement_event);

        self.tx_client.tx::<_, Value>(add_attachement).await?;
        Ok(vec![ToolResultContent::text(format!(
            "Successfully added attachement to message with message_id {}",
            &args.message_id
        ))])
    }
}

#[derive(Debug, Clone)]
struct HulyAiPresenterClient {
    client: reqwest::Client,
    base_url: reqwest::Url,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HulyAiPresenterImage {
    data: String,
    mime_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase", tag = "type")]
enum HulyAiPresenterContent {
    Text(Text),
    Image(HulyAiPresenterImage),
}

impl From<HulyAiPresenterContent> for ToolResultContent {
    fn from(value: HulyAiPresenterContent) -> Self {
        match value {
            HulyAiPresenterContent::Text(text) => ToolResultContent::Text(text),
            HulyAiPresenterContent::Image(image) => ToolResultContent::Image(Image {
                data: image.data,
                format: Some(ContentFormat::Base64),
                media_type: ImageMediaType::from_mime_type(&image.mime_type),
                detail: None,
            }),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
struct HulyAiPresenterResult {
    success: bool,
    data: Option<Vec<HulyAiPresenterContent>>,
    error: Option<String>,
}

impl TryFrom<HulyAiPresenterResult> for Vec<ToolResultContent> {
    type Error = anyhow::Error;

    fn try_from(value: HulyAiPresenterResult) -> std::result::Result<Self, Self::Error> {
        if value.success {
            Ok(value
                .data
                .into_iter()
                .flat_map(|data| data.into_iter())
                .map(ToolResultContent::from)
                .collect())
        } else {
            Err(anyhow!(
                "{}",
                value.error.as_deref().unwrap_or("<unknown error>")
            ))
        }
    }
}

async fn create_presenter_client(
    base_url: reqwest::Url,
    token: SecretString,
) -> Result<HulyAiPresenterClient> {
    let mut default_headers = HeaderMap::new();
    default_headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
    default_headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", token.expose_secret()))?,
    );
    let client = reqwest::Client::builder()
        .default_headers(default_headers)
        .build()?;
    Ok(HulyAiPresenterClient { client, base_url })
}

impl HulyAiPresenterClient {
    async fn get_params_schema(&self) -> Result<HashMap<String, serde_json::Value>> {
        let response = self
            .client
            .get(self.base_url.join("/params-schema.json")?)
            .send()
            .await?;
        Ok(response.json().await?)
    }

    async fn call(&self, name: &str, args: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        tracing::debug!("call {}: {}", name, &args);
        let response = self
            .client
            .post(self.base_url.join(name)?)
            .json(&args)
            .send()
            .await?;
        let r: serde_json::Value = response.json().await?;

        let response: HulyAiPresenterResult = serde_json::from_value(r)?;
        response.try_into()
    }
}

struct HulyPresenterTool {
    client: HulyAiPresenterClient,
    method: String,
}

#[async_trait]
impl ToolImpl for HulyPresenterTool {
    fn name(&self) -> &str {
        &self.method
    }

    async fn call(&mut self, args: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        Ok(self
            .client
            .call(self.method.trim_start_matches("huly_"), args)
            .await?)
    }
}
