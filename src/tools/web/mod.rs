// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{collections::HashMap, time::Duration};

use anyhow::Result;
use async_trait::async_trait;
use itertools::Itertools;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};

use crate::{
    config::{Config, WebSearchProvider},
    context::AgentContext,
    state::AgentState,
    tools::{ToolImpl, ToolSet},
    types::{ImageMediaType, ToolResultContent},
    utils::safe_truncated,
};

pub struct WebToolSet;

impl ToolSet for WebToolSet {
    fn get_name(&self) -> &str {
        "web"
    }

    async fn get_tools<'a>(
        &self,
        config: &'a Config,
        _context: &'a AgentContext,
        _state: &'a AgentState,
    ) -> Vec<Box<dyn ToolImpl>> {
        let mut descriptions =
            serde_json::from_str::<Vec<serde_json::Value>>(include_str!("tools.json"))
                .unwrap()
                .into_iter()
                .map(|v| (v["function"]["name"].as_str().unwrap().to_string(), v))
                .collect::<HashMap<String, serde_json::Value>>();
        vec![
            Box::new(WebFetchTool {
                client: None,
                description: descriptions.remove("web_fetch").unwrap(),
            }),
            Box::new(WebSearchTool {
                client: None,
                config: config.web_search.clone(),
                description: descriptions.remove("web_search").unwrap(),
            }),
        ]
    }

    fn get_system_prompt(&self, _config: &Config) -> String {
        "".to_string()
    }
}

const MAX_LENGTH: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebFetchToolArgs {
    pub url: String,
    #[serde(default)]
    pub max_length: usize,
    #[serde(default)]
    pub start_index: usize,
    #[serde(default)]
    pub raw: bool,
}

pub struct WebFetchTool {
    description: serde_json::Value,
    client: Option<reqwest::Client>,
}

impl WebFetchTool {
    fn format_response(
        args: WebFetchToolArgs,
        content_type: &str,
        text: &str,
    ) -> anyhow::Result<String> {
        let mut result = if args.raw {
            text.to_string()
        } else {
            match content_type {
                "text/plain" => text.to_string(),
                "application/json" => {
                    let json: serde_json::Value = serde_json::from_str(text)?;
                    format!("```json\n{}\n```", serde_json::to_string_pretty(&json)?).to_string()
                }
                _ => {
                    let converter = htmd::HtmlToMarkdownBuilder::new()
                        .skip_tags(vec![
                            "head", "script", "style", "nav", "footer", "header", "link",
                        ])
                        .build();
                    converter.convert(text)?
                }
            }
        }
        .to_owned();
        let max_length = if args.max_length == 0 {
            MAX_LENGTH
        } else {
            args.max_length
        };
        let len = result.chars().count();
        if args.start_index > 0 && args.start_index < len {
            result = result[args.start_index..].to_string();
        }
        if len > max_length {
            result = safe_truncated(&result, max_length);
        }
        Ok(result)
    }
}

#[async_trait]
impl ToolImpl for WebFetchTool {
    fn desciption(&self) -> &serde_json::Value {
        &self.description
    }

    async fn call(&mut self, arguments: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<WebFetchToolArgs>(arguments)?;
        let client = self.client.get_or_insert_with(reqwest::Client::new);
        let response = client
            .get(&args.url)
            .timeout(Duration::from_secs(30))
            .send()
            .await?;

        let content_type = response
            .headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap())
            .unwrap_or("text/html")
            .to_string();

        if content_type.starts_with("text/") {
            let body = response.text().await?;
            Ok(vec![ToolResultContent::text(Self::format_response(
                args,
                &content_type,
                &body,
            )?)])
        } else {
            // try return original image url
            Ok(vec![ToolResultContent::image_url(
                response.url().to_string(),
                ImageMediaType::from_mime_type(&content_type),
            )])
            // let body = response.bytes().await?;
            // Ok(vec![ToolResultContent::image(
            //     base64::engine::general_purpose::STANDARD.encode(&body),
            //     ImageMediaType::from_mime_type(&content_type),
            // )])
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchToolArgs {
    pub query: String,
    #[serde(default)]
    pub count: u16,
    #[serde(default)]
    pub offset: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BraveWebResultItem {
    pub title: String,
    pub url: String,
    pub description: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct BraveWebResult {
    pub results: Vec<BraveWebResultItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BraveResult {
    pub web: BraveWebResult,
}

pub struct WebSearchTool {
    config: WebSearchProvider,
    client: Option<reqwest::Client>,
    description: serde_json::Value,
}

#[async_trait]
impl ToolImpl for WebSearchTool {
    fn desciption(&self) -> &serde_json::Value {
        &self.description
    }

    async fn call(&mut self, arguments: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<WebSearchToolArgs>(arguments)?;
        let client = self.client.get_or_insert_with(reqwest::Client::new);
        match &self.config {
            WebSearchProvider::Brave(search_config) => {
                let url = format!(
                    "https://api.search.brave.com/res/v1/web/search?q={}&count={}&offset={}",
                    utf8_percent_encode(&args.query, NON_ALPHANUMERIC),
                    if args.count == 0 { 10 } else { args.count },
                    args.offset
                );
                tracing::debug!("Perform Brave web search '{}'", url);
                let response = client
                    .get(url)
                    .header("Accept", "application/json")
                    .header("X-Subscription-Token", &search_config.api_key)
                    .send()
                    .await?;
                if response.status() != 200 {
                    anyhow::bail!(
                        "Unexpected status code: {}: {}",
                        response.status(),
                        response.text().await.unwrap()
                    );
                }
                let body = response.text().await?;
                let json: BraveResult = serde_json::from_str(&body)?;
                let converter = htmd::HtmlToMarkdownBuilder::new().build();
                let result = json
                    .web
                    .results
                    .into_iter()
                    .map(|item| {
                        format!(
                            "Title: {}\nDescription: {}\nURL: {}",
                            item.title,
                            converter
                                .convert(&item.description)
                                .unwrap_or(item.description),
                            item.url
                        )
                    })
                    .join("\n\n");
                Ok(vec![ToolResultContent::text(result)])
            }
        }
    }
}
