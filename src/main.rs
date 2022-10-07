use anyhow::anyhow;
use axum::{response::IntoResponse, routing::get, Router};
use axum_tracing_opentelemetry::opentelemetry_tracing_layer;
use config::Config;
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use vaultrs::client::{VaultClient, VaultClientSettingsBuilder};

async fn health() -> impl IntoResponse {
    axum::Json(json!({ "status" : "UP" }))
}

fn app() -> Router {
    Router::new()
        .route("/", get(health))
        .layer(opentelemetry_tracing_layer())
        .layer(TraceLayer::new_for_http())
        .route("/health", get(health))
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::warn!("signal received, starting graceful shutdown");
    opentelemetry::global::shutdown_tracer_provider();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    opentelemetry::global::set_error_handler(|error| {
            ::tracing::error!(target: "opentelemetry", "OpenTelemetry error occurred: {:#}", anyhow!(error));
        })
        .expect("to be able to set error handler");

    let _tracer =
        opentelemetry_datadog::new_pipeline().install_batch(opentelemetry::runtime::Tokio)?;

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "opentelemetry=debug,spot=debug,tower_http=debug".into()),
        ))
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    let settings = Config::builder()
        // Add in `./Settings.toml`
        .add_source(config::File::with_name("spot"))
        .add_source(config::Environment::with_prefix("SPOT"))
        .build()?;

    tracing::info!(
        "settings {:?}",
        settings.try_deserialize::<HashMap<String, String>>()?
    );

    let _client = VaultClient::new(
        VaultClientSettingsBuilder::default()
            .address("https://127.0.0.1:8200")
            .token("TOKEN")
            .build()?,
    )?;

    let app = app();
    let addr = &"0.0.0.0:3000".parse::<SocketAddr>()?;
    tracing::warn!("listening on {}", addr);
    axum::Server::bind(addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}
