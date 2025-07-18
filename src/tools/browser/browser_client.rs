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
    pub tab_id: i32,
    pub body: BrowserRequestMessageType,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OpenTabOptions {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub wait_until_loaded: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScreenshotOptions {
    pub size: (u32, u32),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum BrowserRequestMessageType {
    // Browser control messages
    Close,
    RestoreSession,
    OpenTab {
        options: Option<OpenTabOptions>,
    },
    GetTabs,
    Resize {
        width: u32,
        height: u32,
    },

    // Tab control messages
    CloseTab,
    GetTitle,
    GetUrl,
    Screenshot {
        options: Option<ScreenshotOptions>,
    },
    Navigate {
        url: String,
    },
    MouseMove {
        x: i32,
        y: i32,
    },
    Click {
        x: i32,
        y: i32,
        button: u8,
        down: bool,
    },
    Wheel {
        x: i32,
        y: i32,
        dx: i32,
        dy: i32,
    },
    Key {
        character: u16,
        code: i32,
        windowscode: i32,
        down: bool,
        ctrl: bool,
        shift: bool,
    },
    Char {
        unicode: u16,
    },
    StopVideo,
    StartVideo,
    Reload,
    GoBack,
    GoForward,
    SetFocus(bool),
    GetDOM,
    GetClickableElements,
    ClickElement {
        id: i32,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct WsResponse {
    pub id: String,
    pub tab_id: i32,
    pub body: BrowserResponseMessageType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickableElement {
    pub id: i32,
    pub tag: String,
    pub text: String,
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

    async fn send_notification(
        &mut self,
        tab_id: i32,
        body: BrowserRequestMessageType,
    ) -> Result<()> {
        self.lazy_init().await?;
        let request_id = self
            .id_gen
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            .to_string();

        let request = WsRequest {
            id: request_id.clone(),
            tab_id,
            body,
        };
        tracing::trace!("request: {:?}", request);

        self.sender
            .as_ref()
            .unwrap()
            .write()
            .await
            .send(Message::text(serde_json::to_string(&request)?))
            .await?;
        Ok(())
    }

    async fn send_request(
        &mut self,
        tab_id: i32,
        body: BrowserRequestMessageType,
    ) -> Result<BrowserResponseMessageType> {
        self.lazy_init().await?;
        let request_id = self
            .id_gen
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            .to_string();

        let request = WsRequest {
            id: request_id.clone(),
            tab_id,
            body,
        };
        tracing::trace!("request: {:?}", request);

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
                let response = response?;
                Ok(response.body)
            }
        }
    }

    pub async fn open_url(&mut self, url: &str) -> Result<i32> {
        let resp = self
            .send_request(
                -1,
                BrowserRequestMessageType::OpenTab {
                    options: Some(OpenTabOptions {
                        url: url.to_string(),
                        wait_until_loaded: true,
                    }),
                },
            )
            .await?;
        match resp {
            BrowserResponseMessageType::Tab(tab_id) => Ok(tab_id),
            _ => Err(anyhow::anyhow!("Unexpected response type")),
        }
    }

    pub async fn navigate(&mut self, tab_id: i32, url: &str) -> Result<()> {
        self.send_notification(
            tab_id,
            BrowserRequestMessageType::Navigate {
                url: url.to_string(),
            },
        )
        .await
    }

    pub async fn tabs(&mut self) -> Result<Vec<i32>> {
        let resp = self
            .send_request(-1, BrowserRequestMessageType::GetTabs)
            .await?;
        match resp {
            BrowserResponseMessageType::Tabs(tabs) => Ok(tabs),
            _ => Err(anyhow::anyhow!("Unexpected response type")),
        }
    }

    pub async fn get_clickable_elements(&mut self, tab_id: i32) -> Result<Vec<ClickableElement>> {
        let resp = self
            .send_request(tab_id, BrowserRequestMessageType::GetClickableElements)
            .await?;
        match resp {
            BrowserResponseMessageType::ClickableElements(elements) => Ok(elements),
            _ => Err(anyhow::anyhow!("Unexpected response type")),
        }
    }

    pub async fn click_element(&mut self, tab_id: i32, id: i32) -> Result<()> {
        self.send_notification(tab_id, BrowserRequestMessageType::ClickElement { id })
            .await
    }

    pub async fn get_dom(&mut self, tab_id: i32) -> Result<String> {
        let resp = self
            .send_request(tab_id, BrowserRequestMessageType::GetDOM)
            .await?;
        match resp {
            BrowserResponseMessageType::Dom(dom) => Ok(dom),
            _ => Err(anyhow::anyhow!("Unexpected response type")),
        }
    }

    pub async fn get_title(&mut self, tab_id: i32) -> Result<String> {
        let resp = self
            .send_request(tab_id, BrowserRequestMessageType::GetTitle)
            .await?;
        match resp {
            BrowserResponseMessageType::Title(title) => Ok(title),
            _ => Err(anyhow::anyhow!("Unexpected response type")),
        }
    }

    pub async fn get_url(&mut self, tab_id: i32) -> Result<String> {
        let resp = self
            .send_request(tab_id, BrowserRequestMessageType::GetUrl)
            .await?;
        match resp {
            BrowserResponseMessageType::Url(url) => Ok(url),
            _ => Err(anyhow::anyhow!("Unexpected response type")),
        }
    }

    /// return base64 encoded png
    pub async fn take_screenshot(&mut self, tab_id: i32) -> Result<String> {
        let resp = self
            .send_request(
                tab_id,
                BrowserRequestMessageType::Screenshot {
                    options: Some(ScreenshotOptions { size: (1920, 1080) }),
                },
            )
            .await?;
        match resp {
            BrowserResponseMessageType::Screenshot(screenshot) => Ok(screenshot),
            _ => Err(anyhow::anyhow!("Unexpected response type")),
        }
    }

    pub async fn key(
        &mut self,
        tab_id: i32,
        code: i32,
        down: bool,
        ctrl: bool,
        shift: bool,
    ) -> Result<()> {
        self.send_notification(
            tab_id,
            BrowserRequestMessageType::Key {
                character: 0,
                code,
                windowscode: 0,
                down,
                ctrl,
                shift,
            },
        )
        .await
    }

    pub async fn type_char(&mut self, tab_id: i32, c: char) -> Result<()> {
        self.send_notification(
            tab_id,
            BrowserRequestMessageType::Char { unicode: c as u16 },
        )
        .await
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
            tab_id: -1,
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
    pub async fn take_screenshot(&mut self) -> Result<String> {
        self.browser_client.take_screenshot(self.tab_id).await
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
    async fn test_open_browser() {
        init_logging();
        let mut browser_client = BrowserClientSingleTab::new("ws://localhost:40069/browser");
        browser_client
            .open_url("https://www.google.com/search?q=test")
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_secs(3)).await;

        let screenshot = browser_client.take_screenshot().await.unwrap();
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
