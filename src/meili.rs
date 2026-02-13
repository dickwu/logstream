use std::time::Duration;

use meilisearch_sdk::client::Client;
use tokio::sync::mpsc;
use tokio::time;

use crate::models::LogEntry;

pub const INDEX_NAME: &str = "logs";

/// Initialize the Meilisearch index with the correct settings.
pub async fn init_index(host: &str, key: &str) -> anyhow::Result<()> {
    let client = Client::new(host, Some(key))?;

    // Create index if it doesn't exist
    let task = client.create_index(INDEX_NAME, Some("id")).await?;
    let _ = client.wait_for_task(task, None, None).await;

    let index = client.index(INDEX_NAME);

    // Searchable attributes — what full-text search looks at
    let task = index
        .set_searchable_attributes(["message", "source", "meta"])
        .await?;
    let _ = client.wait_for_task(task, None, None).await;

    // Filterable attributes — used in filter expressions
    let task = index
        .set_filterable_attributes([
            "project",
            "level",
            "traceId",
            "spanId",
            "parentSpanId",
            "environment",
            "timestampMs",
        ])
        .await?;
    let _ = client.wait_for_task(task, None, None).await;

    // Sortable attributes
    let task = index
        .set_sortable_attributes(["timestamp", "timestampMs"])
        .await?;
    let _ = client.wait_for_task(task, None, None).await;

    // Ranking rules: prioritize sort (for timestamp ordering)
    let task = index
        .set_ranking_rules([
            "sort",
            "words",
            "typo",
            "proximity",
            "attribute",
            "exactness",
        ])
        .await?;
    let _ = client.wait_for_task(task, None, None).await;

    // Pagination
    let task = index
        .set_pagination(meilisearch_sdk::settings::PaginationSetting {
            max_total_hits: 10000,
        })
        .await?;
    let _ = client.wait_for_task(task, None, None).await;

    tracing::info!("Meilisearch index '{}' configured", INDEX_NAME);
    Ok(())
}

/// Background task that batches log entries and flushes to Meilisearch.
///
/// Receives entries via an mpsc channel, buffers them, and flushes either
/// when the buffer hits BATCH_SIZE or every FLUSH_INTERVAL — whichever
/// comes first.
pub struct MeiliBatcher {
    client: Client,
    rx: mpsc::UnboundedReceiver<LogEntry>,
}

const BATCH_SIZE: usize = 200;
const FLUSH_INTERVAL: Duration = Duration::from_millis(250);

impl MeiliBatcher {
    pub fn new(host: &str, key: &str) -> anyhow::Result<(Self, mpsc::UnboundedSender<LogEntry>)> {
        let client = Client::new(host, Some(key))?;
        let (tx, rx) = mpsc::unbounded_channel();
        Ok((Self { client, rx }, tx))
    }

    /// Run the batcher loop. Call this in a spawned task.
    pub async fn run(mut self) {
        let index = self.client.index(INDEX_NAME);
        let mut buffer: Vec<LogEntry> = Vec::with_capacity(BATCH_SIZE);
        let mut interval = time::interval(FLUSH_INTERVAL);
        interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                // Receive a log entry
                entry = self.rx.recv() => {
                    match entry {
                        Some(e) => {
                            buffer.push(e);
                            if buffer.len() >= BATCH_SIZE {
                                flush(&index, &mut buffer).await;
                            }
                        }
                        None => {
                            // Channel closed, flush remaining and exit
                            if !buffer.is_empty() {
                                flush(&index, &mut buffer).await;
                            }
                            tracing::info!("Meili batcher shutting down");
                            return;
                        }
                    }
                }
                // Periodic flush
                _ = interval.tick() => {
                    if !buffer.is_empty() {
                        flush(&index, &mut buffer).await;
                    }
                }
            }
        }
    }
}

async fn flush(index: &meilisearch_sdk::indexes::Index, buffer: &mut Vec<LogEntry>) {
    let count = buffer.len();
    let docs: Vec<LogEntry> = buffer.drain(..).collect();

    match index.add_documents(&docs, Some("id")).await {
        Ok(_task) => {
            tracing::debug!("Flushed {} logs to Meilisearch", count);
        }
        Err(e) => {
            tracing::error!("Meilisearch flush error: {:?}", e);
            // TODO: dead-letter queue or retry
        }
    }
}

/// Build a Meilisearch filter string from query parameters.
pub fn build_filter(
    project: Option<&str>,
    level: Option<&str>,
    trace_id: Option<&str>,
    environment: Option<&str>,
    since: Option<&str>,
) -> Option<String> {
    let mut clauses: Vec<String> = Vec::new();

    if let Some(p) = project {
        clauses.push(format!("project = \"{}\"", p));
    }
    if let Some(l) = level {
        clauses.push(format!("level = \"{}\"", l));
    }
    if let Some(t) = trace_id {
        clauses.push(format!("traceId = \"{}\"", t));
    }
    if let Some(e) = environment {
        clauses.push(format!("environment = \"{}\"", e));
    }
    if let Some(s) = since {
        if let Some(ms) = parse_duration(s) {
            let cutoff = chrono::Utc::now().timestamp_millis() - ms;
            clauses.push(format!("timestampMs > {}", cutoff));
        }
    }

    if clauses.is_empty() {
        None
    } else {
        Some(clauses.join(" AND "))
    }
}

/// Parse a human-friendly duration string like "5m", "1h", "2d" into milliseconds.
pub fn parse_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 2 {
        return None;
    }

    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: i64 = num_str.parse().ok()?;

    let multiplier = match unit {
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };

    Some(num * multiplier)
}
