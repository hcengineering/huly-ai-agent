// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
use reqwest::Url;
use secrecy::SecretString;
use serde::Deserialize;

const DEFAULT_CONFIG: &str = include_str!("config.yml");
const LOCAL_CONFIG_FILE: &str = "config-local.yml";

#[derive(Debug, Deserialize, Clone)]
pub enum ProviderKind {
    OpenAI,
    OpenRouter,
    Anthropic,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub log_level: String,
    pub huly: HulyConfig,
    pub provider: ProviderKind,
    pub provider_api_key: Option<SecretString>,
    pub model: String,
    pub user_instructions: String,
    pub workspace: PathBuf,
    pub mcp: Option<HashMap<String, McpConfig>>,
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
    pub log_level: String,
    pub group_id: String,
    pub topics: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PersonConfig {
    pub email: String,
    pub password: SecretString,
    #[allow(dead_code)]
    pub sex: String,
    #[allow(dead_code)]
    pub age: String,
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

impl Config {
    pub fn new(data_dir: &str) -> Result<Self> {
        let mut builder = config::Config::builder()
            .add_source(config::File::from_str(
                DEFAULT_CONFIG,
                config::FileFormat::Yaml,
            ))
            .add_source(config::Environment::with_prefix("AI_AGENT"));

        if Path::new(LOCAL_CONFIG_FILE).exists() {
            builder = builder.add_source(config::File::with_name(LOCAL_CONFIG_FILE));
        }

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
