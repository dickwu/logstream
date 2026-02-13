use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use meilisearch_sdk::client::Client;
use tower_http::cors::{Any, CorsLayer};

use crate::config::Config;
use crate::meili::MeiliBatcher;
use crate::routes::{self, AppState};
use crate::subscribers::SubscriberManager;

/// Start the log collector server.
pub async fn run(cfg: Config) -> anyhow::Result<()> {
    // Initialize Meilisearch batcher
    let (batcher, meili_tx) = MeiliBatcher::new(&cfg.meili_host, &cfg.meili_key)?;

    // Meilisearch client for queries
    let meili_client = Client::new(&cfg.meili_host, Some(&cfg.meili_key))?;

    // Subscriber manager
    let subscribers = SubscriberManager::new();

    // Shared state
    let state = Arc::new(AppState {
        meili_tx,
        subscribers,
        meili_client,
    });

    // CORS
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Routes
    let app = Router::new()
        .route("/health", get(routes::health))
        .route("/ingest", post(routes::ingest))
        .route("/search", get(routes::search))
        .route("/projects", get(routes::projects))
        .route("/trace/{trace_id}", get(routes::trace))
        .route("/errors", get(routes::errors))
        .route("/ws", get(routes::ws_handler))
        .layer(cors)
        .with_state(state);

    // Spawn batcher
    tokio::spawn(batcher.run());

    // Start server
    let addr = format!("0.0.0.0:{}", cfg.port);
    tracing::info!("Logstream collector listening on {}", addr);
    tracing::info!("  Meilisearch: {}", cfg.meili_host);
    tracing::info!("  Endpoints:");
    tracing::info!("    POST /ingest        — HTTP log ingestion");
    tracing::info!("    GET  /ws            — WebSocket (ingest + subscribe)");
    tracing::info!("    GET  /search        — Query logs");
    tracing::info!("    GET  /projects      — Project breakdown");
    tracing::info!("    GET  /trace/:id     — Trace timeline");
    tracing::info!("    GET  /errors        — Error summary");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
