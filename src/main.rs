// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::fs;
use std::panic::set_hook;
use std::panic::take_hook;
use std::path::Path;
use std::str::FromStr;

use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use hulyrs::services::account::LoginParams;
use hulyrs::services::account::SelectWorkspaceParams;
use hulyrs::services::account::WorkspaceKind;
use hulyrs::services::jwt::ClaimsBuilder;
use hulyrs::services::transactor::comm::CreateMessageEvent;
use hulyrs::services::transactor::document::DocumentClient;
use hulyrs::services::transactor::document::FindOptionsBuilder;
use hulyrs::ServiceFactory;
use secrecy::ExposeSecret;
use tokio::sync::mpsc;
use tracing::Subscriber;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::reload;
use tracing_subscriber::reload::Handle;
use tracing_subscriber::Layer;
use tracing_subscriber::Registry;

use self::config::Config;
use crate::agent::Agent;
use crate::context::AgentContext;
use crate::context::MessagesContext;
use crate::task::task_multiplexer;
use crate::task::Task;

use clap::Parser;
use tokio::select;
use tokio::signal::*;

mod agent;
mod channel_log;
mod config;
mod context;
mod huly;
mod providers;
mod state;
mod task;
mod templates;
mod tools;
mod types;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to data directory
    #[arg(short, long, default_value = "data")]
    data: String,
}

type LogHandle = Handle<Vec<Box<dyn Layer<Registry> + Send + Sync>>, Registry>;

fn init_logger(config: &Config) -> Result<LogHandle> {
    let layers = default_layers(config)?;
    // wrap the vec in a reload layer

    let (layers, reload_handle) = reload::Layer::new(layers);
    let subscriber = tracing_subscriber::registry().with(layers);

    match tracing::subscriber::set_global_default(subscriber) {
        Ok(_) => Ok(reload_handle),
        Err(e) => Err(e.into()),
    }
}

fn default_layers<S>(config: &Config) -> Result<Vec<Box<dyn Layer<S> + Send + Sync + 'static>>>
where
    S: Subscriber,
    for<'a> S: LookupSpan<'a>,
{
    let console_layer = tracing_subscriber::fmt::layer()
        .with_ansi(true)
        .with_target(true)
        .with_writer(std::io::stdout)
        .with_filter(
            tracing_subscriber::filter::Targets::default()
                .with_default(tracing::Level::WARN)
                .with_target(
                    "huly_ai_agent",
                    tracing::Level::from_str(&config.log_level).unwrap(),
                ),
        )
        .boxed();
    Ok(vec![console_layer])
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

    let log_handle = init_logger(&config)?;

    #[cfg(not(feature = "mcp"))]
    if config.mcp.is_some() {
        bail!("Config contains mcp section but mcp feature is not enabled");
    }
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Started");

    let data_dir = Path::new(&args.data);
    if !data_dir.exists() {
        fs::create_dir_all(data_dir)?;
    }

    tracing::debug!("account_service_url: {}", config.huly.account_service);
    tracing::debug!("kafka_bootstrap: {}", config.huly.kafka.bootstrap);

    let hulyrs_config = hulyrs::ConfigBuilder::default()
        .account_service(config.huly.account_service.clone())
        .kafka_bootstrap_servers(vec![config.huly.kafka.bootstrap.clone()])
        .token_secret("secret")
        .log(tracing::Level::from_str(&config.log_level).unwrap())
        .build()?;

    let service_factory = ServiceFactory::new(hulyrs_config);
    let account_client = service_factory
        .new_account_client(&ClaimsBuilder::default().guest_account().build()?)
        .with_context(|| "Failed to create guestaccount client")?;

    let login_info = account_client
        .login(&LoginParams {
            email: config.huly.person.email.clone(),
            password: config.huly.person.password.expose_secret().to_string(),
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
            kind: WorkspaceKind::Internal,
            external_regions: Vec::default(),
        })
        .await?;

    let tx_client = service_factory.new_transactor_client_from_token(
        ws_info.endpoint,
        workspace.workspace.uuid,
        ws_info.base.token.unwrap(),
    )?;

    let query = serde_json::json!({
        "personUuid": login_info.account,
    });
    let options = FindOptionsBuilder::default().project("_id").build()?;

    let person = tx_client
        .find_one::<_, serde_json::Value>("contact:class:Person", query, &options)
        .await?
        .unwrap();

    let person_id = person["_id"].as_str().unwrap();
    let social_id = login_info.social_id.unwrap();

    let message_context = MessagesContext {
        config: config.clone(),
        workspace_uuid: workspaces[0].workspace.uuid,
        account_uuid: login_info.account,
        person_id: person_id.to_string(),
    };
    let agent_context = AgentContext {
        social_id: social_id.clone(),
        tx_client,
    };

    let channel_log_handle = if let Some(channel_id) = &config.log_channel {
        let (log_sender, log_receiver) =
            tokio::sync::mpsc::unbounded_channel::<CreateMessageEvent>();
        let event_publisher = service_factory.new_kafka_publisher("hulygun")?;
        let level = tracing::Level::from_str(&config.log_level)?;
        log_handle.modify(|filter| {
            (*filter).push(
                channel_log::HulyChannelLogWriter::new(
                    level,
                    log_sender,
                    social_id,
                    channel_id.clone(),
                )
                .boxed(),
            );
        })?;
        Some(channel_log::run_channel_log_worker(
            event_publisher,
            workspaces[0].workspace.uuid,
            log_receiver,
        ))
    } else {
        None
    };

    tracing::info!("Logged in as {}", message_context.account_uuid);

    let (messages_sender, messages_receiver) = mpsc::unbounded_channel();
    let (task_sender, task_receiver) = tokio::sync::mpsc::unbounded_channel::<Task>();

    let task_multiplexer = task_multiplexer(messages_receiver, task_sender);
    let messages_listener = huly::streaming::worker(message_context, messages_sender);

    let agent = Agent::new(&args.data, config)?;
    let agent_handle = agent.run(task_receiver, agent_context);

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
        _ = async { channel_log_handle.expect("crash here").await }, if channel_log_handle.is_some() => {
            tracing::info!("Channel log worker terminated");
        }
    }

    tracing::debug!("Shutting down");

    Ok(())
}
