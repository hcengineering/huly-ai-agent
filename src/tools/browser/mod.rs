// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{
    config::{BrowserConfig, Config},
    context::AgentContext,
    state::AgentState,
    tools::{ToolImpl, ToolSet},
    types::ToolResultContent,
};

mod browser_client;

type BrowserClientRef = Arc<RwLock<browser_client::BrowserClientSingleTab>>;
pub struct BrowserToolSet {
    browser_client: Option<BrowserClientRef>,
}

#[derive(Deserialize)]
struct ProfileResponse {
    status: bool,
    data: Option<ProfileResponseData>,
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ProfileResponseData {
    address: String,
}

impl BrowserToolSet {
    async fn get_browser_url(browser_config: &BrowserConfig) -> Result<String> {
        let client = reqwest::Client::new();
        let profile_url = format!(
            "{}/profiles/{}/cef",
            browser_config.bootstrap_url, browser_config.profile_name
        );
        let resp = client
            .get(&profile_url)
            .send()
            .await
            .context(format!("Get url error: {profile_url}"))?
            .error_for_status()?;
        let resp_text = resp.text().await?;

        let resp = match serde_json::from_str::<ProfileResponse>(&resp_text) {
            Ok(resp) => resp,
            Err(e) => bail!("Failed to parse JSON: {}\n{}", e, resp_text),
        };
        if resp.status && resp.data.is_some() {
            Ok(resp.data.unwrap().address)
        } else {
            bail!(
                resp.error
                    .unwrap_or(serde_json::Value::String("unknown error".to_string()))
            );
        }
    }

    pub async fn new(browser_config: &BrowserConfig) -> Self {
        let browser_client = match Self::get_browser_url(browser_config).await {
            Ok(url) => Some(Arc::new(RwLock::new(
                browser_client::BrowserClientSingleTab::new(&url),
            ))),
            Err(e) => {
                tracing::error!("Failed to get browser url: {}", e);
                None
            }
        };
        Self { browser_client }
    }
}

impl ToolSet for BrowserToolSet {
    fn get_tools<'a>(
        &self,
        _config: &'a Config,
        _context: &'a AgentContext,
        _state: &'a AgentState,
    ) -> Vec<Box<dyn ToolImpl>> {
        if let Some(browser_client) = &self.browser_client {
            vec![
                Box::new(OpenPageTool {
                    client: browser_client.clone(),
                }),
                Box::new(GetClickableElementsTool {
                    client: browser_client.clone(),
                }),
                Box::new(ClickElementTool {
                    client: browser_client.clone(),
                }),
                Box::new(ScreenshotTool {
                    client: browser_client.clone(),
                }),
                Box::new(PressEnterTool {
                    client: browser_client.clone(),
                }),
                Box::new(TypeTextTool {
                    client: browser_client.clone(),
                }),
            ]
        } else {
            tracing::warn!("Browser is not configured");
            vec![]
        }
    }

    fn get_tool_descriptions(&self, _config: &Config) -> Vec<serde_json::Value> {
        serde_json::from_str(include_str!("tools.json")).unwrap()
    }

    fn get_system_prompt(&self, _config: &Config) -> String {
        "".to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenUrlToolArgs {
    pub url: String,
}

struct OpenPageTool {
    client: BrowserClientRef,
}

#[async_trait]
impl ToolImpl for OpenPageTool {
    fn name(&self) -> &str {
        "browser-open-page"
    }

    async fn call(&mut self, arguments: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<OpenUrlToolArgs>(arguments)?;
        self.client.write().await.open_url(&args.url).await?;
        tokio::time::sleep(Duration::from_secs(3)).await;
        let elements = self.client.write().await.get_clickable_elements().await?;
        let elements = elements
            .iter()
            .map(|e| {
                format!(
                    "[{id}] <{tag}>{text}</{tag}>",
                    id = e.id,
                    tag = e.tag,
                    text = e.text
                )
            })
            .collect::<Vec<String>>()
            .join("\n");
        Ok(vec![ToolResultContent::text(format!(
            "Opened page at {}\n\nClickable elements:\n{elements}",
            args.url
        ))])
    }
}

struct GetClickableElementsTool {
    client: BrowserClientRef,
}

#[async_trait]
impl ToolImpl for GetClickableElementsTool {
    fn name(&self) -> &str {
        "browser-get-clickable-elements"
    }

    async fn call(&mut self, _arguments: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        let elements = self.client.write().await.get_clickable_elements().await?;
        let elements = elements
            .iter()
            .map(|e| {
                format!(
                    "[{id}] <{tag}>{text}</{tag}>",
                    id = e.id,
                    tag = e.tag,
                    text = e.text
                )
            })
            .collect::<Vec<String>>()
            .join("\n");
        Ok(vec![ToolResultContent::text(format!(
            "Clickable elements:\n{elements}"
        ))])
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClickElementToolArgs {
    index: i32,
}

struct ClickElementTool {
    client: BrowserClientRef,
}

#[async_trait]
impl ToolImpl for ClickElementTool {
    fn name(&self) -> &str {
        "browser-click-element"
    }

    async fn call(&mut self, arguments: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<ClickElementToolArgs>(arguments)?;
        self.client.write().await.click_element(args.index).await?;
        Ok(vec![ToolResultContent::text(format!(
            "Clicked element at index {}",
            args.index
        ))])
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScreenshotToolArgs {
    #[serde(default)]
    dimension: Option<String>,
}

struct ScreenshotTool {
    client: BrowserClientRef,
}

#[async_trait]
impl ToolImpl for ScreenshotTool {
    fn name(&self) -> &str {
        "browser-take-screenshot"
    }

    async fn call(&mut self, arguments: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<ScreenshotToolArgs>(arguments)?;
        let (width, height) = args
            .dimension
            .and_then(|s| {
                s.split_once('x').map(|(width, height)| {
                    (width.parse().unwrap_or(480), height.parse().unwrap_or(270))
                })
            })
            .unwrap_or((480, 270));
        let screenshot = self
            .client
            .write()
            .await
            .take_screenshot(width, height)
            .await?;
        Ok(vec![ToolResultContent::image(
            screenshot,
            Some(crate::types::ImageMediaType::PNG),
        )])
    }
}

struct PressEnterTool {
    client: BrowserClientRef,
}

#[async_trait]
impl ToolImpl for PressEnterTool {
    fn name(&self) -> &str {
        "browser-press-enter"
    }

    async fn call(&mut self, _arguments: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        self.client
            .write()
            .await
            .key(13, true, false, false)
            .await?;
        tokio::time::sleep(Duration::from_millis(100)).await;
        self.client
            .write()
            .await
            .key(13, false, false, false)
            .await?;
        Ok(vec![ToolResultContent::text("Enter pressed".to_string())])
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TypeTextArgs {
    text: String,
}

struct TypeTextTool {
    client: BrowserClientRef,
}

#[async_trait]
impl ToolImpl for TypeTextTool {
    fn name(&self) -> &str {
        "browser-type-text"
    }

    async fn call(&mut self, arguments: serde_json::Value) -> Result<Vec<ToolResultContent>> {
        let args = serde_json::from_value::<TypeTextArgs>(arguments)?;
        for c in args.text.chars() {
            self.client.write().await.type_char(c).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(vec![ToolResultContent::text(format!(
            "Text '{}' typed",
            args.text
        ))])
    }
}
