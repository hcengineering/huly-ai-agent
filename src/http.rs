// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use actix_cors::Cors;
use actix_web::{App, HttpResponse, HttpServer, dev::ServerHandle, middleware, web};
use anyhow::Result;
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{config::Config, huly::streaming::types::CommunicationEvent};

pub fn server(
    config: &Config,
    sender: mpsc::UnboundedSender<CommunicationEvent>,
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
            .wrap(middleware::Logger::default())
            .wrap(cors)
            .route("/event", web::post().to(post_event))
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
    event: web::Json<CommunicationEvent>,
) -> Result<HttpResponse, actix_web::Error> {
    let event = event.into_inner();
    tracing::trace!(event = ?event, "Received event");
    if let Err(e) = sender.send(event) {
        tracing::error!(error = ?e, "Failed to send event");
    }
    Ok(HttpResponse::Ok().finish())
}
