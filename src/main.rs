// Copyright В© 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::fs;
use std::panic::set_hook;
use std::panic::take_hook;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

use crate::agent::Agent;
use crate::task::Task;

use self::config::Config;
// use crate::agent::AgentControlEvent;
// use crate::agent::AgentOutputEvent;
use clap::Parser;

mod agent;
mod config;
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

fn init_logger() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(std::io::stdout)
                .with_target(true)
                .with_filter(
                    tracing_subscriber::filter::Targets::new()
                        .with_target("hyper_util::client", tracing::Level::INFO)
                        .with_default(tracing::Level::DEBUG),
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

#[tokio::main]
async fn main() -> Result<()> {
    init_panic_hook();
    let args = Args::parse();

    init_logger();

    tracing::info!("Start");
    let config = match Config::new(&args.data) {
        Ok(config) => config,
        Err(e) => {
            println!("Error: Failed to load config");
            return Err(e);
        }
    };
    let data_dir = Path::new(&args.data);
    if !data_dir.exists() {
        fs::create_dir_all(data_dir)?;
    }

    let agent = Agent::new(&args.data, config.clone())?;
    let (task_sender, task_receiver) = tokio::sync::mpsc::unbounded_channel::<Task>();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(3)).await;
        task_sender
            .send(Task::direct_question("Konstantin", "What is you version?"))
            .unwrap();
    });
    agent.run(task_receiver).await?;

    Ok(())
}
