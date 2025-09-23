// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::panic::set_hook;
use std::panic::take_hook;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use huly::fetch_server_config;
use hulyrs::ServiceFactory;
use hulyrs::services::account::LoginParams;
use hulyrs::services::account::SelectWorkspaceParams;
use hulyrs::services::account::WorkspaceKind;
use hulyrs::services::core::storage::WithoutStructure;
use hulyrs::services::jwt::ClaimsBuilder;
use hulyrs::services::transactor::TransactorClient;
use hulyrs::services::transactor::backend::http::HttpBackend;
use hulyrs::services::transactor::document::DocumentClient;
use hulyrs::services::transactor::document::FindOptionsBuilder;
use opentelemetry::trace::TracerProvider;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use secrecy::ExposeSecret;
use serde_json::json;
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tracing::Level;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use uuid::Uuid;

use self::config::Config;
use crate::agent::Agent;
use crate::config::AgentMode;
use crate::context::AgentContext;
use crate::context::HulyAccountInfo;
use crate::context::MessagesContext;
use crate::huly::blob::BlobClient;
use crate::huly::types::CommunicationDirect;
use crate::huly::types::Person;
use crate::huly::typing::TypingClient;
use crate::task::Task;
use crate::task::task_multiplexer;
use crate::tools::command::process_registry::ProcessRegistry;

use clap::Parser;
use tokio::select;
use tokio::signal::*;

mod agent;
mod config;
mod context;
mod database;
mod http;
mod huly;
mod memory;
mod otel;
mod providers;
mod scheduler;
mod state;
mod task;
mod templates;
mod tools;
mod types;
mod utils;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to data directory
    #[arg(short, long, default_value = "data")]
    data: String,
}

fn init_logger(config: &Config) -> Result<()> {
    otel::init_meters(config);
    let package_name = env!("CARGO_PKG_NAME").replace('-', "_");

    let console_layer = tracing_subscriber::fmt::layer()
        .with_ansi(true)
        .with_target(true)
        .with_writer(std::io::stdout)
        .with_filter(
            Targets::default()
                .with_default(tracing::Level::WARN)
                .with_target(&package_name, config.log_level),
        );

    let tracer_layer = otel::tracer_provider(config).map(|provider| {
        let filter = Targets::default()
            .with_default(Level::WARN)
            .with_target(&package_name, config.log_level);

        OpenTelemetryLayer::new(provider.tracer(package_name.clone())).with_filter(filter)
    });

    let logger_layer = otel::logger_provider(config)
        .as_ref()
        .map(OpenTelemetryTracingBridge::new)
        .with_filter(
            Targets::default()
                .with_default(Level::WARN)
                .with_target(&package_name, config.log_level),
        );

    tracing_subscriber::registry()
        .with(console_layer)
        .with(tracer_layer)
        .with(logger_layer)
        .try_init()?;
    Ok(())
}

fn init_panic_hook() {
    let original_hook = take_hook();
    set_hook(Box::new(move |panic_info| {
        // intentionally ignore errors here since we're already in a panic
        let backtrace = std::backtrace::Backtrace::capture();
        tracing::error!("{}, {:#?}", panic_info, backtrace);
        original_hook(panic_info);
        std::process::exit(1);
    }));
}

#[cfg(unix)]
async fn wait_interrupt() -> Result<()> {
    let mut term = unix::signal(unix::SignalKind::terminate())?;
    let mut int = unix::signal(unix::SignalKind::interrupt())?;
    let mut quit = unix::signal(unix::SignalKind::quit())?;

    select! {
        _ = term.recv() => {
            tracing::info!("Received SIGTERM");
        }

        _ = int.recv() => {
            tracing::info!("Received SIGINT");
        }

        _ = quit.recv() => {
            tracing::info!("Received SIGQUIT");
        }
    };

    Ok(())
}

#[cfg(windows)]
async fn wait_interrupt() -> Result<()> {
    let mut term = windows::ctrl_close()?;
    let mut int = windows::ctrl_c()?;
    let mut quit = windows::ctrl_shutdown()?;

    select! {
        _ = term.recv() => {
            tracing::info!("Received CTRL+CLOSE");
        }

        _ = int.recv() => {
            tracing::info!("Received CTRL+C");
        }

        _ = quit.recv() => {
            tracing::info!("Received Shutdown");
        }
    };

    Ok(())
}

async fn employee_login(
    config: &Config,
    service_factory: &ServiceFactory,
) -> Result<(HulyAccountInfo, TransactorClient<HttpBackend>)> {
    let account_client = service_factory
        .new_account_client_without_user()
        .with_context(|| "Failed to create guestaccount client")?;

    let login_info = account_client
        .login(&LoginParams {
            email: config.huly.person.as_ref().unwrap().email.clone(),
            password: config
                .huly
                .person
                .as_ref()
                .unwrap()
                .password
                .expose_secret()
                .to_string(),
        })
        .await?;

    let Some(token) = login_info.token else {
        bail!("Account is not confirmed, no token provided");
    };

    let account_client = service_factory
        .new_account_client_from_token(login_info.account, token.clone())
        .with_context(|| "Failed to create agent account client")?;

    let workspaces = account_client.get_user_workspaces().await?;
    let workspace = workspaces[0].clone();
    let ws_info = account_client
        .select_workspace(&SelectWorkspaceParams {
            workspace_url: workspace.workspace.url,
            kind: WorkspaceKind::External,
            external_regions: Vec::default(),
        })
        .await?;
    tracing::info!(
        "Entered workspace {} ({:?})",
        ws_info.workspace,
        ws_info.workspace_url
    );

    let tx_client = service_factory.new_transactor_client_from_token(
        ws_info.endpoint,
        workspace.workspace.uuid,
        ws_info.base.token.clone().unwrap(),
    )?;

    let query = json!({
        "personUuid": login_info.account,
    });
    let options = FindOptionsBuilder::default()
        .project("_id")
        .project("name")
        .build();

    let person = tx_client
        .find_one::<WithoutStructure<Person>, serde_json::Value>(query, &options)
        .await?
        .unwrap();

    let person_id = person.data["_id"].as_str().unwrap();
    let person_name = person.data["name"].as_str().unwrap();
    let social_id = login_info.social_id.unwrap();

    Ok((
        HulyAccountInfo {
            account_uuid: login_info.account,
            person_name: person_name.to_string(),
            token: ws_info.base.token.unwrap().into(),
            person_id: person_id.to_string(),
            social_id,
            workspace: workspaces[0].workspace.uuid,
            control_card_id: None,
            time_zone: chrono_tz::UTC,
        },
        tx_client.clone(),
    ))
}

async fn assistant_login(
    service_factory: &ServiceFactory,
    account_uuid: &str,
) -> Result<(HulyAccountInfo, TransactorClient<HttpBackend>)> {
    let account_uuid = Uuid::parse_str(account_uuid)?;

    let account_client = service_factory.new_account_client(
        &ClaimsBuilder::default()
            .account(account_uuid)
            .service("huly-assistant")
            .build()?,
    )?;

    let social_id = account_client
        .find_social_id_by_social_key(&format!("huly-assistant:{}", account_uuid), true)
        .await?
        .unwrap();

    let account_info = account_client.get_account_info(&account_uuid).await?;
    let workspaces = account_client.get_user_workspaces().await?;
    let workspace = workspaces[0].clone();
    let ws_info = account_client
        .select_workspace(&SelectWorkspaceParams {
            workspace_url: workspace.workspace.url,
            kind: WorkspaceKind::External,
            external_regions: Vec::default(),
        })
        .await?;
    tracing::info!(
        "Entered workspace {} ({:?})",
        ws_info.workspace,
        ws_info.workspace_url
    );
    let token = ws_info.base.token.unwrap();
    let tx_client = service_factory.new_transactor_client_from_token(
        ws_info.endpoint,
        workspace.workspace.uuid,
        token.clone(),
    )?;

    let query = json!({
        "personUuid": account_uuid,
    });
    let options = FindOptionsBuilder::default()
        .project("_id")
        .project("name")
        .build();

    let person = tx_client
        .find_one::<WithoutStructure<Person>, serde_json::Value>(query, &options)
        .await?
        .unwrap();
    let person_id = person.data["_id"].as_str().unwrap();
    let person_name = person.data["name"].as_str().unwrap();

    let control_card_id = tx_client
        .find_all::<CommunicationDirect, serde_json::Value>(
            json!({}),
            &FindOptionsBuilder::default().build(),
        )
        .await?
        .value
        .iter()
        .find_map(|card| {
            if card.members.len() == 1 {
                Some(card.doc.id.clone())
            } else {
                None
            }
        });

    if control_card_id.is_none() {
        tracing::warn!("No direct control chat found");
    }

    Ok((
        HulyAccountInfo {
            account_uuid,
            person_name: person_name.to_string(),
            token: token.into(),
            person_id: person_id.to_string(),
            social_id,
            workspace: workspace.workspace.uuid,
            control_card_id,
            time_zone: account_info
                .timezone
                .unwrap_or("UTC".to_string())
                .parse()
                .unwrap_or(chrono_tz::UTC),
        },
        tx_client.clone(),
    ))
}

#[tokio::main]
async fn main() -> Result<()> {
    init_panic_hook();
    dotenv::dotenv().ok();
    let args = Args::parse();

    let config = match Config::new(&args.data) {
        Ok(config) => config,
        Err(e) => {
            println!("Error: Failed to load config");
            return Err(e);
        }
    };

    init_logger(&config)?;

    #[cfg(not(feature = "mcp"))]
    if config.mcp.is_some() {
        bail!("Config contains mcp section but mcp feature is not enabled");
    }
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Started");

    let data_dir = Path::new(&args.data);
    if !data_dir.exists() {
        fs::create_dir_all(data_dir)?;
    }

    tracing::debug!("base_url: {}", config.huly.base_url);
    tracing::debug!("kafka_bootstrap: {}", config.huly.kafka.bootstrap);

    let server_config = fetch_server_config(config.huly.base_url.clone()).await?;
    let hulyrs_config = hulyrs::ConfigBuilder::default()
        .account_service(server_config.accounts_url.clone())
        .kafka_bootstrap_servers(vec![config.huly.kafka.bootstrap.clone()])
        .pulse_service(server_config.pulse_url.clone())
        .token_secret("secret".to_string())
        .log(config.log_level)
        .build()?;

    let service_factory = ServiceFactory::new(hulyrs_config);

    let (account_info, tx_client) = match &config.agent_mode {
        AgentMode::Employee => employee_login(&config, &service_factory).await?,
        AgentMode::PersonalAssistant(account_uuid) => {
            assistant_login(&service_factory, account_uuid).await?
        }
    };

    let direct_cards = tx_client
        .find_all::<WithoutStructure<CommunicationDirect>, serde_json::Value>(
            json!({}),
            &FindOptionsBuilder::default().project("_id").build(),
        )
        .await?
        .value
        .iter()
        .map(|card| card.data["_id"].as_str().unwrap().to_string())
        .collect::<HashSet<String>>();

    let blob_client = BlobClient::new(
        &server_config,
        account_info.workspace,
        account_info.token.clone(),
    )?;
    let process_registry = ProcessRegistry::default();
    let process_registry = Arc::new(RwLock::new(process_registry));

    let db_client = database::DbClient::new(&args.data, &config).await?;
    let pulse_client =
        service_factory.new_pulse_client(account_info.workspace, account_info.token.clone())?;
    let typing_client = TypingClient::new(pulse_client, &account_info.person_id);

    let message_context = MessagesContext {
        config: config.clone(),
        server_config: server_config.clone(),
        tx_client: tx_client.clone(),
        workspace_uuid: account_info.workspace,
        account_uuid: account_info.account_uuid,
        person_id: account_info.person_id.clone(),
        card_info_cache: HashMap::new(),
        space_info_cache: HashMap::new(),
        person_info_cache: HashMap::new(),
    };
    let agent_context = AgentContext {
        account_info: account_info.clone(),
        process_registry: process_registry.clone(),
        tx_client,
        blob_client,
        typing_client,
        db_client: db_client.clone(),
        tools_context: None,
        tools_system_prompt: None,
    };

    tracing::info!("Logged in as {}", message_context.account_uuid);

    let (messages_sender, messages_receiver) = mpsc::unbounded_channel();
    let (task_sender, task_receiver) = tokio::sync::mpsc::unbounded_channel::<Task>();
    let (memory_task_sender, memory_task_receiver) = tokio::sync::mpsc::unbounded_channel::<Task>();

    let task_multiplexer = task_multiplexer(
        messages_receiver,
        task_sender.clone(),
        config.agent_mode.clone(),
        account_info,
    );
    let messages_listener =
        huly::streaming::worker(message_context, messages_sender.clone(), direct_cards);

    let agent = Agent::new(config.clone())?;
    let agent_handle = agent.run(task_receiver, memory_task_sender, agent_context);
    let memory_worker_handler =
        memory::memory_worker(&config, memory_task_receiver, db_client.clone())?;
    let scheduler_handler = scheduler::scheduler(&config, db_client, task_sender.clone())?;
    let (http_server, http_server_handle) = http::server(&config, messages_sender)?;

    select! {
        _ = wait_interrupt() => {
        }
        res = task_multiplexer => {
            if let Err(e) = res {
                tracing::error!("Task multiplexer terminated with error: {:?}", e);
            } else {
                tracing::info!("Task multiplexer terminated");
            }
        }
        res = messages_listener => {
            if let Err(e) = res {
                tracing::error!("Messages listener error: {:?}", e);
            }
            tracing::info!("Messages listener terminated");
        }
        res = agent_handle => {
            if let Err(e) = res {
                tracing::error!("Agent error: {:?}", e);
            }
            tracing::info!("Agent terminated");
        }
        res = http_server => {
            if let Err(e) = res {
                tracing::error!("Http server error: {:?}", e);
            }
            tracing::info!("Http server terminated");
        }
    }

    tracing::debug!("Shutting down");
    http_server_handle.stop(true).await;
    process_registry.write().await.stop().await;
    memory_worker_handler.abort();
    scheduler_handler.abort();
    Ok(())
}
