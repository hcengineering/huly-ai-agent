// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use actix_cors::Cors;
use actix_web::{App, HttpResponse, HttpServer, dev::ServerHandle, middleware, web};
use anyhow::Result;
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{
    communication::{AgentState, ScheduledTask, types::CommunicationEvent},
    config::Config,
    database::DbClient,
};

pub fn server(
    config: &Config,
    sender: mpsc::UnboundedSender<CommunicationEvent>,
    db_client: DbClient,
    activity_sender: mpsc::UnboundedSender<()>,
) -> Result<(JoinHandle<Result<(), std::io::Error>>, ServerHandle)> {
    let socket = std::net::SocketAddr::new(
        config.http_api.bind_host.as_str().parse()?,
        config.http_api.bind_port,
    );

    tracing::info!(bind = ?socket, "Starting http server");

    let server = HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header()
            .supports_credentials()
            .max_age(3600);

        App::new()
            .app_data(web::Data::new(sender.clone()))
            .app_data(web::Data::new(db_client.clone()))
            .app_data(web::Data::new(activity_sender.clone()))
            .wrap(middleware::Logger::default())
            .wrap(cors)
            .route("/event", web::post().to(post_event))
            .route("/state", web::get().to(state))
            .route(
                "/status",
                web::get().to(async || {
                    format!(
                        "OK {}/{}",
                        env!("CARGO_PKG_NAME"),
                        env!("CARGO_PKG_VERSION")
                    )
                }),
            )
    })
    .disable_signals()
    .bind(socket)?
    .run();

    let server_handle = server.handle();
    let server = tokio::spawn(server);

    Ok((server, server_handle))
}

async fn post_event(
    sender: web::Data<mpsc::UnboundedSender<CommunicationEvent>>,
    activity_sender: web::Data<mpsc::UnboundedSender<()>>,
    event: web::Json<CommunicationEvent>,
) -> Result<HttpResponse, actix_web::Error> {
    let event = event.into_inner();
    tracing::trace!(event = ?event, "Received event");
    activity_sender.send(()).ok();
    if let Err(e) = sender.send(event) {
        tracing::error!(error = ?e, "Failed to send event");
    }
    Ok(HttpResponse::Ok().finish())
}

async fn state(db_client: web::Data<DbClient>) -> Result<HttpResponse, actix_web::Error> {
    let db_client = db_client.into_inner();
    let has_unfinished_tasks = !db_client.unfinished_tasks().await.is_empty();
    let upcoming_jobs = db_client.get_scheduler().await.ok().unwrap_or_default();
    let next_scheduled = upcoming_jobs
        .into_iter()
        .min_by(|x, y| x.1.cmp(&y.1))
        .map(|item| ScheduledTask {
            task_kind: item.0,
            schedule: item.1,
        });
    Ok(HttpResponse::Ok().json(AgentState {
        has_actve_task: has_unfinished_tasks,
        next_scheduled,
    }))
}
