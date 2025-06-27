// Copyright В© 2025 Huly Labs. Use of this source code is governed by the MIT license.

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
use hulyrs::services::transactor::document::DocumentClient;
use hulyrs::services::transactor::document::FindOptionsBuilder;
use hulyrs::ServiceFactory;
use secrecy::ExposeSecret;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::mpsc;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

use self::config::Config;
use crate::agent::Agent;
use crate::context::AgentContext;
use crate::context::MessagesContext;
use crate::task::task_multiplexer;
use crate::task::Task;

use tokio::select;
use tokio::signal::*;
// use crate::agent::AgentControlEvent;
// use crate::agent::AgentOutputEvent;
use clap::Parser;

mod agent;
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

fn init_logger(config: &Config) {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(true)
                .with_writer(std::io::stdout)
                .with_target(true)
                .with_filter(
                    tracing_subscriber::filter::Targets::default()
                        .with_default(tracing::Level::WARN)
                        .with_target(
                            "huly_ai_agent",
                            tracing::Level::from_str(&config.log_level).unwrap(),
                        ),
                ),
        )
        .init()
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

#[derive(Deserialize, Serialize, Debug)]
pub struct TestQuery {}

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
    let args = Args::parse();

    let config = match Config::new(&args.data) {
        Ok(config) => config,
        Err(e) => {
            println!("Error: Failed to load config");
            return Err(e);
        }
    };

    init_logger(&config);

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Started");

    let data_dir = Path::new(&args.data);
    if !data_dir.exists() {
        fs::create_dir_all(data_dir)?;
    }

    let hulyrs_config = hulyrs::ConfigBuilder::default()
        .account_service(config.huly.account_service.clone())
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

    println!("{:?}", login_info);
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
        social_id,
        tx_client,
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
        _ = task_multiplexer => {
            tracing::info!("Task multiplexer terminated");
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
    }

    tracing::debug!("Shutting down");

    Ok(())
}
