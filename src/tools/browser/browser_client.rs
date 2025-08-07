// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{
    collections::HashMap,
    fmt::Debug,
    sync::{Arc, atomic::AtomicU64},
    time::Duration,
};

use anyhow::Result;
use futures::{SinkExt, StreamExt, stream::SplitSink};
use serde::{Deserialize, Serialize};
use tokio::{
    net::TcpStream,
    select,
    sync::{RwLock, oneshot},
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Serialize, Deserialize)]
struct WsRequest {
    pub id: String,
    #[serde(flatten)]
    pub params: WsRequestParams,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "camelCase")]
pub enum WsRequestParams {
    OpenTab {
        url: String,
        wait_until_loaded: bool,
        width: u32,
        height: u32,
    },
    Navigate {
        tab: i32,
        url: String,
        wait_until_loaded: bool,
    },
    GetTabs {},
    GetClickableElements {
        tab: i32,
    },
    ClickElement {
        tab: i32,
        element_id: i32,
    },
    GetDOM {
        tab: i32,
    },
    GetTitle {
        tab: i32,
    },
    GetUrl {
        tab: i32,
    },
    Screenshot {
        tab: i32,
        width: u32,
        height: u32,
    },
    Key {
        tab: i32,
        character: u16,
        windowscode: i32,
        code: i32,
        down: bool,
        ctrl: bool,
        shift: bool,
    },
    Char {
        tab: i32,
        unicode: u16,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct WsResponse {
    pub id: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickableElement {
    pub id: i32,
    pub tag: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenTabResult {
    pub id: i32,
    pub url: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabsResult {
    pub tabs: Vec<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickableElementsResult {
    pub elements: Vec<ClickableElement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DOMResult {
    pub dom: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TitleResult {
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotResult {
    pub screenshot: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrlResult {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmptySuccessResult {
    pub success: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum BrowserResponseMessageType {
    Session(Vec<String>),
    Tab(i32),
    Tabs(Vec<i32>),
    Title(String),
    Url(String),
    Screenshot(String),
    Dom(String),
    ClickableElements(Vec<ClickableElement>),
}
type RequestsMap = Arc<RwLock<HashMap<String, oneshot::Sender<WsResponse>>>>;
type MessagesSender = RwLock<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>;

pub struct BrowserClient {
    ws_url: String,
    sender: Option<MessagesSender>,
    requests: RequestsMap,
    id_gen: AtomicU64,
}

impl BrowserClient {
    pub fn new(ws_url: &str) -> Self {
        Self {
            ws_url: ws_url.to_string(),
            sender: None,
            requests: Arc::new(RwLock::new(HashMap::new())),
            id_gen: AtomicU64::new(1),
        }
    }

    async fn lazy_init(&mut self) -> Result<()> {
        if self.sender.is_some() {
            return Ok(());
        }

        tracing::debug!("Connecting to browser websocket: {}", self.ws_url);
        let (stream, _) = tokio_tungstenite::connect_async(self.ws_url.clone()).await?;
        let (ws_tx, mut ws_rx) = stream.split();
        self.sender = Some(RwLock::new(ws_tx));
        {
            let requests = Arc::clone(&self.requests);
            tokio::spawn(async move {
                while let Some(result) = ws_rx.next().await {
                    //tracing::trace!("Received message: {:?}", result);
                    match result {
                        Ok(message) => {
                            if let Err(e) = Self::handle_ws_message(&requests, message).await {
                                tracing::error!("{}", e);
                            }
                        }
                        Err(e) => {
                            tracing::error!("{}", e);
                        }
                    }
                }
            });
        }
        Ok(())
    }

    async fn handle_ws_message(requests: &RequestsMap, message: Message) -> Result<()> {
        match message {
            Message::Text(text) => {
                let response: WsResponse = serde_json::from_str(&text)?;
                if let Some(tx) = requests.write().await.remove(&response.id) {
                    let _ = tx.send(response);
                } else {
                    tracing::warn!("No request found for id: {}", response.id);
                }
            }
            Message::Binary(_) => tracing::warn!("Binary message received"),
            Message::Ping(_) => tracing::warn!("Ping message received"),
            Message::Pong(_) => tracing::warn!("Pong message received"),
            Message::Close(_) => tracing::warn!("Close message received"),
            Message::Frame(_) => tracing::warn!("Frame message received"),
        }
        Ok(())
    }

    async fn send_request<T>(&mut self, params: WsRequestParams) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        self.lazy_init().await?;
        let request_id = self
            .id_gen
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            .to_string();

        let request = WsRequest {
            id: request_id.clone(),
            params,
        };
        tracing::trace!("request: {}", serde_json::to_string_pretty(&request)?);

        self.sender
            .as_ref()
            .unwrap()
            .write()
            .await
            .send(Message::text(serde_json::to_string(&request)?))
            .await?;

        let (tx, rx) = oneshot::channel();

        let mut requests = self.requests.write().await;
        requests.insert(request_id.clone(), tx);
        drop(requests);

        select! {
            _ = tokio::time::sleep(REQUEST_TIMEOUT) => {
                tracing::warn!("Request timed out");
                Err(anyhow::anyhow!("Request timed out, no response received, request_id: {}", request_id))
            }
            response = rx => {
                tracing::trace!("response: {:?}", response);
                match response? {
                    WsResponse { result: Some(result), .. } => serde_json::from_value(result).map_err(|e| anyhow::anyhow!("{}", e)),
                    WsResponse { error: Some(error), .. } => Err(anyhow::anyhow!("{}", error)),
                    _ => Err(anyhow::anyhow!("Unexpected response type")),
                }
            }
        }
    }

    pub async fn open_url(&mut self, url: &str) -> Result<i32> {
        let resp = self
            .send_request::<OpenTabResult>(WsRequestParams::OpenTab {
                url: url.to_string(),
                wait_until_loaded: true,
                width: 1920,
                height: 1080,
            })
            .await?;
        Ok(resp.id)
    }

    pub async fn navigate(&mut self, tab_id: i32, url: &str) -> Result<()> {
        self.send_request::<EmptySuccessResult>(WsRequestParams::Navigate {
            tab: tab_id,
            wait_until_loaded: true,
            url: url.to_string(),
        })
        .await?;
        Ok(())
    }

    pub async fn tabs(&mut self) -> Result<Vec<i32>> {
        let resp = self
            .send_request::<TabsResult>(WsRequestParams::GetTabs {})
            .await?;
        Ok(resp.tabs)
    }

    pub async fn get_clickable_elements(&mut self, tab: i32) -> Result<Vec<ClickableElement>> {
        let resp = self
            .send_request::<ClickableElementsResult>(WsRequestParams::GetClickableElements { tab })
            .await?;
        Ok(resp.elements)
    }

    pub async fn click_element(&mut self, tab: i32, element_id: i32) -> Result<()> {
        self.send_request::<EmptySuccessResult>(WsRequestParams::ClickElement { tab, element_id })
            .await?;
        Ok(())
    }

    pub async fn get_dom(&mut self, tab: i32) -> Result<String> {
        let resp = self
            .send_request::<DOMResult>(WsRequestParams::GetDOM { tab })
            .await?;
        Ok(resp.dom)
    }

    pub async fn get_title(&mut self, tab: i32) -> Result<String> {
        let resp = self
            .send_request::<TitleResult>(WsRequestParams::GetTitle { tab })
            .await?;
        Ok(resp.title)
    }

    pub async fn get_url(&mut self, tab: i32) -> Result<String> {
        let resp = self
            .send_request::<UrlResult>(WsRequestParams::GetUrl { tab })
            .await?;
        Ok(resp.url)
    }

    /// return base64 encoded png
    pub async fn take_screenshot(&mut self, tab: i32, width: u32, height: u32) -> Result<String> {
        let resp = self
            .send_request::<ScreenshotResult>(WsRequestParams::Screenshot { tab, width, height })
            .await?;
        Ok(resp.screenshot)
    }

    pub async fn key(
        &mut self,
        tab: i32,
        code: i32,
        down: bool,
        ctrl: bool,
        shift: bool,
    ) -> Result<()> {
        self.send_request::<EmptySuccessResult>(WsRequestParams::Key {
            tab,
            character: 0,
            code,
            windowscode: 0,
            down,
            ctrl,
            shift,
        })
        .await?;
        Ok(())
    }

    pub async fn type_char(&mut self, tab: i32, c: char) -> Result<()> {
        self.send_request::<EmptySuccessResult>(WsRequestParams::Char {
            tab,
            unicode: c as u16,
        })
        .await?;
        Ok(())
    }
}

pub struct BrowserClientSingleTab {
    tab_id: i32,
    browser_client: BrowserClient,
}

#[allow(dead_code)]
impl BrowserClientSingleTab {
    pub fn new(ws_url: &str) -> Self {
        Self {
            tab_id: 1,
            browser_client: BrowserClient::new(ws_url),
        }
    }

    pub async fn open_url(&mut self, url: &str) -> Result<()> {
        let tabs = self.browser_client.tabs().await?;
        if tabs.is_empty() {
            self.tab_id = self.browser_client.open_url(url).await?;
        } else {
            self.tab_id = tabs[0];
            self.browser_client.navigate(self.tab_id, url).await?;
        }
        Ok(())
    }

    pub async fn get_clickable_elements(&mut self) -> Result<Vec<ClickableElement>> {
        self.browser_client
            .get_clickable_elements(self.tab_id)
            .await
    }

    pub async fn click_element(&mut self, id: i32) -> Result<()> {
        self.browser_client.click_element(self.tab_id, id).await
    }

    pub async fn get_dom(&mut self) -> Result<String> {
        self.browser_client.get_dom(self.tab_id).await
    }

    pub async fn get_title(&mut self) -> Result<String> {
        self.browser_client.get_title(self.tab_id).await
    }

    pub async fn get_url(&mut self) -> Result<String> {
        self.browser_client.get_url(self.tab_id).await
    }

    /// return base64 encoded png
    pub async fn take_screenshot(&mut self, width: u32, height: u32) -> Result<String> {
        self.browser_client
            .take_screenshot(self.tab_id, width, height)
            .await
    }

    pub async fn key(&mut self, code: i32, down: bool, ctrl: bool, shift: bool) -> Result<()> {
        self.browser_client
            .key(self.tab_id, code, down, ctrl, shift)
            .await
    }

    pub async fn type_char(&mut self, c: char) -> Result<()> {
        self.browser_client.type_char(self.tab_id, c).await
    }
}

#[cfg(test)]
mod tests {
    use tracing_subscriber::{Layer, layer::SubscriberExt, util::SubscriberInitExt};

    use super::*;
    use base64::prelude::*;

    fn init_logging() {
        let console_layer = tracing_subscriber::fmt::layer()
            .with_ansi(true)
            .with_target(true)
            .with_writer(std::io::stdout)
            .with_filter(
                tracing_subscriber::filter::Targets::default()
                    .with_default(tracing::Level::INFO)
                    .with_target("huly_ai_agen", tracing::Level::TRACE),
            );
        tracing_subscriber::registry().with(console_layer).init();
    }

    #[tokio::test]
    #[ignore]
    async fn test_open_browser() {
        init_logging();
        let mut browser_client = BrowserClientSingleTab::new("ws://localhost:40063/browser");
        browser_client
            .open_url("https://www.google.com/search?q=test")
            .await
            .unwrap();

        let title = browser_client.get_title().await.unwrap();
        assert!(title.contains("www.google.com"));

        let url = browser_client.get_url().await.unwrap();
        assert!(url.contains("https://www.google.com/"));

        tokio::time::sleep(Duration::from_secs(3)).await;

        let screenshot = browser_client.take_screenshot(800, 600).await.unwrap();
        std::fs::write(
            "screenshot.png",
            BASE64_STANDARD.decode(screenshot).unwrap(),
        )
        .unwrap();

        // assert!(browser_client.tabs().await.unwrap().is_empty());
        // let tab_id = browser_client
        //     .open_url("https://www.google.com")
        //     .await
        //     .unwrap();
        // assert!(tab_id > 0);
        // assert!(
        //     !browser_client
        //         .get_clickable_elements(tab_id)
        //         .await
        //         .unwrap()
        //         .is_empty()
        // );
        // assert!(browser_client.close_tab(tab_id).await.is_ok());
        // assert!(browser_client.tabs().await.unwrap().is_empty());
    }
}
