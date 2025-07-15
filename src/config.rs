// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{Result, anyhow};
use reqwest::Url;
use secrecy::SecretString;
use serde::{Deserialize, Deserializer, de::Error};

const DEFAULT_CONFIG: &str = include_str!("config.yml");
const LOCAL_CONFIG_FILE: &str = "config-local.yml";

#[derive(Debug, Deserialize, Clone)]
pub enum ProviderKind {
    OpenAI,
    OpenRouter,
    Anthropic,
}

fn deserialize_log_level<'de, D>(deserializer: D) -> Result<tracing::Level, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    tracing::Level::from_str(&s).map_err(|e| D::Error::custom(e.to_string()))
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(deserialize_with = "deserialize_log_level")]
    pub log_level: tracing::Level,
    pub huly: HulyConfig,
    pub provider: ProviderKind,
    pub provider_api_key: Option<SecretString>,
    pub model: String,
    pub user_instructions: String,
    pub workspace: PathBuf,
    pub log_channel: Option<String>,
    pub mcp: Option<HashMap<String, McpConfig>>,
    pub voyageai_api_key: SecretString,
    pub voyageai_model: String,
    pub voyageai_dimensions: u16,
    pub web_search: WebSearchProvider,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HulyConfig {
    pub kafka: KafkaConfig,
    pub account_service: Url,
    pub person: PersonConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct KafkaConfig {
    pub bootstrap: String,
    pub group_id: String,
    pub topics: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PersonConfig {
    pub email: String,
    pub password: SecretString,
    pub name: String,
    #[allow(dead_code)]
    pub sex: String,
    #[allow(dead_code)]
    pub age: String,
    pub personality: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "transport", rename_all = "lowercase")]
pub enum McpTransportConfig {
    Sse { url: String, version: String },
}

#[derive(Debug, Deserialize, Clone)]
pub struct McpConfig {
    #[serde(flatten)]
    pub transport: McpTransportConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WebSearchBraveConfig {
    pub api_key: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum WebSearchProvider {
    Brave(WebSearchBraveConfig),
}

impl Config {
    pub fn new(data_dir: &str) -> Result<Self> {
        let mut builder = config::Config::builder()
            .add_source(config::File::from_str(
                DEFAULT_CONFIG,
                config::FileFormat::Yaml,
            ))
            .add_source(
                config::Environment::with_prefix("AGENT")
                    .prefix_separator("_")
                    .separator("__"),
            );

        if Path::new(LOCAL_CONFIG_FILE).exists() {
            builder = builder.add_source(config::File::with_name(LOCAL_CONFIG_FILE));
        }

        fs::create_dir_all(std::path::Path::new(data_dir).join("ws"))?;

        builder
            .set_override(
                "workspace",
                std::path::Path::new(data_dir).join("ws").to_str().unwrap(),
            )?
            .build()?
            .try_deserialize()
            .map_err(|e| anyhow!("Failed to deserialize config: {}", e))
    }
}
