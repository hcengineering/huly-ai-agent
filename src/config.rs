// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use reqwest::Url;
use secrecy::SecretString;
use serde::{
    Deserialize, Deserializer,
    de::{self, Error, Visitor},
};

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
    pub otel: OtelMode,
    pub agent_mode: AgentMode,
    pub http_api: HttpApiConfig,
    pub model: String,
    pub provider: ProviderKind,
    pub provider_api_key: Option<SecretString>,
    pub huly: HulyConfig,
    pub user_instructions: String,
    pub workspace: PathBuf,
    pub mcp: Option<HashMap<String, McpConfig>>,
    pub voyageai_api_key: SecretString,
    pub voyageai_model: String,
    pub voyageai_dimensions: u16,
    pub web_search: WebSearchProvider,
    pub browser: Option<BrowserConfig>,
    pub memory: MemoryConfig,
    pub jobs: Vec<JobDefinition>,
    pub tasks: HashMap<TaskKind, TaskConfig>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct HttpApiConfig {
    pub bind_host: String,
    pub bind_port: u16,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    Employee,
    PersonalAssistant(String),
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Sleep,
    FollowChat,
    AssistantChat,
    AssistantTask,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct TaskConfig {
    /// available tools by wildcards
    pub tools: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MemoryConfig {
    pub extract_model: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum OtelMode {
    On,
    Stdout,
    Off,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HulyConfig {
    pub kafka: KafkaConfig,
    pub base_url: Url,
    #[serde(default)]
    pub person: Option<PersonConfig>,
    pub ignored_channels: HashSet<String>,
    #[serde(default)]
    pub presenter_url: Option<Url>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct KafkaTopics {
    pub transactions: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct KafkaConfig {
    pub bootstrap: String,
    pub group_id: String,
    pub topics: KafkaTopics,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PersonConfig {
    pub email: String,
    pub password: SecretString,
    pub name: String,
    pub sex: String,
    pub age: String,
    pub rgb_role: RgbRole,
    pub rgb_opponents: Vec<(String, RgbRole)>,
    pub personality: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum RgbRole {
    Red,
    Green,
    Blue,
}

impl Display for RgbRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RgbRole::Red => write!(f, "Challenger"),
            RgbRole::Green => write!(f, "Advocate"),
            RgbRole::Blue => write!(f, "Mediator"),
        }
    }
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

#[derive(Debug, Deserialize, Clone)]
pub struct BrowserConfig {
    pub bootstrap_url: String,
    pub profile_name: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct JobDefinition {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: JobKind,
    pub schedule: JobSchedule,
    #[serde(
        rename = "time_spread_mins",
        deserialize_with = "duration_from_mins",
        default
    )]
    pub time_spread: Duration,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    MemoryMantainance,
    Sleep,
}

#[derive(Debug, Clone)]
pub struct JobSchedule(cron::Schedule);

impl JobSchedule {
    pub fn new(schedule: &str) -> Result<Self> {
        Ok(Self(cron::Schedule::from_str(schedule)?))
    }

    pub fn source(&self) -> &str {
        self.0.source()
    }

    pub fn upcoming(&self) -> DateTime<Utc> {
        self.0.upcoming(Utc).next().unwrap_or(Utc::now())
    }
}

struct JobScheduleVisitor;
impl<'de> Visitor<'de> for JobScheduleVisitor {
    type Value = JobSchedule;
    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a valid cron expression")
    }
    fn visit_str<E>(self, v: &str) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(JobSchedule(cron::Schedule::from_str(v).map_err(E::custom)?))
    }
}

impl<'de> Deserialize<'de> for JobSchedule {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(JobScheduleVisitor)
    }
}

pub fn duration_from_mins<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    u64::deserialize(deserializer).map(|mins| Duration::from_secs(mins * 60))
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
