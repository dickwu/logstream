use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use futures::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::mpsc;

use crate::meili;
use crate::models::{IngestPayload, LogEntry, SearchParams};
use crate::subscribers::{SubscriberFilter, SubscriberManager};

/// Shared state passed to all route handlers.
pub struct AppState {
    pub meili_tx: mpsc::UnboundedSender<LogEntry>,
    pub subscribers: Arc<SubscriberManager>,
    pub meili_client: meilisearch_sdk::client::Client,
}

// ────────────────────────────────────────────
// POST /ingest — HTTP log ingestion
// ────────────────────────────────────────────

pub async fn ingest(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<IngestPayload>,
) -> impl IntoResponse {
    let entries = payload.into_entries();
    let count = entries.len();

    for entry in entries {
        let entry = entry.normalize();

        // Broadcast to live subscribers immediately
        state.subscribers.broadcast(&entry);

        // Send to Meilisearch batcher
        let _ = state.meili_tx.send(entry);
    }

    (StatusCode::ACCEPTED, Json(json!({ "accepted": count })))
}

// ────────────────────────────────────────────
// GET /search — query logs via Meilisearch
// ────────────────────────────────────────────

pub async fn search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    let index = state.meili_client.index(meili::INDEX_NAME);
    let query_str = params.q.as_deref().unwrap_or("");
    let limit = params.limit.unwrap_or(20).min(200);

    let filter = meili::build_filter(
        params.project.as_deref(),
        params.level.as_deref(),
        params.trace_id.as_deref(),
        params.request_id.as_deref(),
        params.environment.as_deref(),
        params.since.as_deref(),
    );

    let mut search_query = index.search();
    search_query.with_query(query_str);
    search_query.with_limit(limit);
    search_query.with_sort(&["timestamp:desc"]);
    search_query.with_facets(meilisearch_sdk::search::Selectors::Some(&[
        "project", "level",
    ]));

    if let Some(ref f) = filter {
        search_query.with_filter(f);
    }

    match search_query.execute::<serde_json::Value>().await {
        Ok(results) => {
            let response = json!({
                "totalHits": results.estimated_total_hits,
                "facets": results.facet_distribution,
                "hits": results.hits.iter().map(|h| &h.result).collect::<Vec<_>>(),
            });
            (StatusCode::OK, Json(response))
        }
        Err(e) => {
            tracing::error!("Search error: {:?}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("{}", e) })),
            )
        }
    }
}

// ────────────────────────────────────────────
// GET /projects — faceted project breakdown
// ────────────────────────────────────────────

pub async fn projects(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let index = state.meili_client.index(meili::INDEX_NAME);

    let mut search_query = index.search();
    search_query.with_query("");
    search_query.with_limit(0);
    search_query.with_facets(meilisearch_sdk::search::Selectors::Some(&[
        "project",
        "level",
        "environment",
    ]));

    match search_query.execute::<serde_json::Value>().await {
        Ok(results) => {
            let response = json!({
                "totalLogs": results.estimated_total_hits,
                "facets": results.facet_distribution,
            });
            Json(response).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{}", e) })),
        )
            .into_response(),
    }
}

// ────────────────────────────────────────────
// GET /trace/:trace_id — full trace timeline
// ────────────────────────────────────────────

pub async fn trace(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(trace_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let index = state.meili_client.index(meili::INDEX_NAME);

    let filter = format!("traceId = \"{}\"", trace_id);

    let mut search_query = index.search();
    search_query.with_query("");
    search_query.with_filter(&filter);
    search_query.with_sort(&["timestamp:asc"]);
    search_query.with_limit(500);

    match search_query.execute::<serde_json::Value>().await {
        Ok(results) => {
            let hits: Vec<&serde_json::Value> = results.hits.iter().map(|h| &h.result).collect();
            let projects: std::collections::HashSet<String> = hits
                .iter()
                .filter_map(|h| h.get("project").and_then(|p| p.as_str()).map(String::from))
                .collect();

            let response = json!({
                "traceId": trace_id,
                "eventCount": hits.len(),
                "projects": projects,
                "timeline": hits,
            });
            Json(response).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{}", e) })),
        )
            .into_response(),
    }
}

// ────────────────────────────────────────────
// GET /request/:request_id — full request timeline
// ────────────────────────────────────────────

pub async fn request(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(request_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let index = state.meili_client.index(meili::INDEX_NAME);

    let filter = format!("requestId = \"{}\"", request_id);

    let mut search_query = index.search();
    search_query.with_query("");
    search_query.with_filter(&filter);
    search_query.with_sort(&["timestamp:asc"]);
    search_query.with_limit(500);

    match search_query.execute::<serde_json::Value>().await {
        Ok(results) => {
            let hits: Vec<&serde_json::Value> = results.hits.iter().map(|h| &h.result).collect();
            let projects: std::collections::HashSet<String> = hits
                .iter()
                .filter_map(|h| h.get("project").and_then(|p| p.as_str()).map(String::from))
                .collect();

            let response = json!({
                "requestId": request_id,
                "eventCount": hits.len(),
                "projects": projects,
                "timeline": hits,
            });
            Json(response).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{}", e) })),
        )
            .into_response(),
    }
}

// ────────────────────────────────────────────
// GET /errors — error summary with facets
// ────────────────────────────────────────────

pub async fn errors(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    let index = state.meili_client.index(meili::INDEX_NAME);

    let mut filter_parts = vec!["level = \"error\" OR level = \"fatal\"".to_string()];

    if let Some(ref p) = params.project {
        filter_parts.push(format!("project = \"{}\"", p));
    }
    if let Some(ref s) = params.since {
        if let Some(ms) = meili::parse_duration(s) {
            let cutoff = chrono::Utc::now().timestamp_millis() - ms;
            filter_parts.push(format!("timestampMs > {}", cutoff));
        }
    }

    // Wrap the OR clause in parens when combining with AND
    let filter = if filter_parts.len() > 1 {
        let first = format!("({})", filter_parts[0]);
        let rest: Vec<String> = filter_parts[1..].to_vec();
        let mut all = vec![first];
        all.extend(rest);
        all.join(" AND ")
    } else {
        filter_parts[0].clone()
    };

    let query_str = params.q.as_deref().unwrap_or("");

    let mut search_query = index.search();
    search_query.with_query(query_str);
    search_query.with_filter(&filter);
    search_query.with_sort(&["timestamp:desc"]);
    search_query.with_limit(params.limit.unwrap_or(30).min(100));
    search_query.with_facets(meilisearch_sdk::search::Selectors::Some(&["project"]));

    match search_query.execute::<serde_json::Value>().await {
        Ok(results) => {
            let response = json!({
                "totalErrors": results.estimated_total_hits,
                "byProject": results.facet_distribution,
                "errors": results.hits.iter().map(|h| &h.result).collect::<Vec<_>>(),
            });
            Json(response).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{}", e) })),
        )
            .into_response(),
    }
}

// ────────────────────────────────────────────
// GET /health
// ────────────────────────────────────────────

pub async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "subscribers": state.subscribers.count(),
    }))
}

// ────────────────────────────────────────────
// WebSocket handler — supports both ingest and subscribe modes
// ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct WsParams {
    pub mode: Option<String>,
    pub projects: Option<String>,
    pub levels: Option<String>,
    #[serde(rename = "traceId")]
    pub trace_id: Option<String>,
}

use serde::Deserialize;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<WsParams>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, params, state))
}

async fn handle_ws(socket: WebSocket, params: WsParams, state: Arc<AppState>) {
    let mode = params.mode.as_deref().unwrap_or("ingest");

    match mode {
        "subscribe" => handle_subscribe(socket, params, state).await,
        _ => handle_ingest(socket, state).await,
    }
}

/// Subscribe mode: stream matching logs to the client in real time.
async fn handle_subscribe(socket: WebSocket, params: WsParams, state: Arc<AppState>) {
    let filter = SubscriberFilter {
        projects: params
            .projects
            .map(|p| p.split(',').map(String::from).collect())
            .unwrap_or_default(),
        levels: params
            .levels
            .map(|l| l.split(',').map(String::from).collect())
            .unwrap_or_default(),
        trace_id: params.trace_id,
    };

    let (sub_id, mut rx) = state.subscribers.subscribe(filter.clone());
    tracing::info!(sub_id, ?filter, "Subscriber connected");

    let (mut ws_tx, mut ws_rx) = socket.split();

    // Send connected message
    let connected_msg = serde_json::to_string(&json!({
        "type": "connected",
        "subscriberId": sub_id,
        "filters": {
            "projects": filter.projects,
            "levels": filter.levels,
            "traceId": filter.trace_id,
        }
    }))
    .unwrap();
    let _ = ws_tx.send(Message::Text(connected_msg.into())).await;

    // Forward logs from subscriber channel to WebSocket
    let send_task = tokio::spawn(async move {
        while let Some(payload) = rx.recv().await {
            if ws_tx.send(Message::Text(payload.into())).await.is_err() {
                break;
            }
        }
    });

    // Wait for client disconnect (ignore incoming messages)
    while let Some(Ok(_)) = ws_rx.next().await {}

    // Cleanup
    state.subscribers.unsubscribe(sub_id);
    send_task.abort();
    tracing::info!(sub_id, "Subscriber disconnected");
}

/// Ingest mode: receive logs from the client via WebSocket.
async fn handle_ingest(socket: WebSocket, state: Arc<AppState>) {
    let (mut _ws_tx, mut ws_rx) = socket.split();

    while let Some(Ok(msg)) = ws_rx.next().await {
        if let Message::Text(text) = msg {
            match serde_json::from_str::<IngestPayload>(&text) {
                Ok(payload) => {
                    for entry in payload.into_entries() {
                        let entry = entry.normalize();
                        state.subscribers.broadcast(&entry);
                        let _ = state.meili_tx.send(entry);
                    }
                }
                Err(e) => {
                    tracing::warn!("Invalid WS message: {:?}", e);
                }
            }
        }
    }
}
