// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde::Deserialize;

const CONFIG_FILE: &str = "config.yml";
const LOCAL_CONFIG_FILE: &str = "config-local.yml";

#[derive(Debug, Deserialize, Clone)]
pub enum ProviderKind {
    OpenAI,
    OpenRouter,
    LMStudio,
    Anthropic,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub provider: ProviderKind,
    pub provider_api_key: Option<String>,
    pub model: String,
    pub user_instructions: String,
    pub workspace: PathBuf,
}

impl Config {
    pub fn new(data_dir: &str) -> Result<Self> {
        let mut builder = config::Config::builder()
            .add_source(config::File::with_name(
                std::path::Path::new(data_dir)
                    .join(CONFIG_FILE)
                    .to_str()
                    .ok_or(anyhow!("Invalid path"))?,
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
