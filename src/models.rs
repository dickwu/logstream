use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single log entry — the core data model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// Unique ID (ULID, auto-generated if not provided)
    #[serde(default = "generate_id")]
    pub id: String,

    /// ISO 8601 timestamp
    #[serde(default = "now")]
    pub timestamp: String,

    /// Unix milliseconds (for Meilisearch numeric range filters)
    #[serde(default)]
    pub timestamp_ms: i64,

    /// Project name: "frontend", "api-server", "auth-service", etc.
    #[serde(default = "default_project")]
    pub project: String,

    /// Log level
    #[serde(default = "default_level")]
    pub level: LogLevel,

    /// Log message
    #[serde(default)]
    pub message: String,

    // --- Tracing ---
    /// Correlation ID across services
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,

    /// Specific operation ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,

    /// Parent operation ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,

    // --- Context ---
    /// Arbitrary metadata (serialized as JSON string for Meili search)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,

    /// Source file/component
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,

    /// Environment: dev, staging, prod
    #[serde(default = "default_env")]
    pub environment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Debug => write!(f, "debug"),
            LogLevel::Info => write!(f, "info"),
            LogLevel::Warn => write!(f, "warn"),
            LogLevel::Error => write!(f, "error"),
            LogLevel::Fatal => write!(f, "fatal"),
        }
    }
}

impl LogEntry {
    /// Normalize the entry: fill in defaults, compute timestamp_ms
    pub fn normalize(mut self) -> Self {
        if self.id.is_empty() {
            self.id = generate_id();
        }
        if self.timestamp.is_empty() {
            self.timestamp = now();
        }
        // Compute timestamp_ms from the ISO string
        if self.timestamp_ms == 0 {
            self.timestamp_ms = self
                .timestamp
                .parse::<DateTime<Utc>>()
                .map(|dt| dt.timestamp_millis())
                .unwrap_or_else(|_| Utc::now().timestamp_millis());
        }
        // Serialize meta to a string representation for full-text search
        self
    }
}

/// Incoming payload: single entry or batch
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum IngestPayload {
    Single(LogEntry),
    Batch(Vec<LogEntry>),
}

impl IngestPayload {
    pub fn into_entries(self) -> Vec<LogEntry> {
        match self {
            IngestPayload::Single(e) => vec![e],
            IngestPayload::Batch(v) => v,
        }
    }
}

/// Query parameters for the /search endpoint
#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub q: Option<String>,
    pub project: Option<String>,
    pub level: Option<String>,
    pub trace_id: Option<String>,
    pub environment: Option<String>,
    pub since: Option<String>,
    pub limit: Option<usize>,
}

// --- Defaults ---

fn generate_id() -> String {
    ulid::Ulid::new().to_string()
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn default_project() -> String {
    "unknown".into()
}

fn default_level() -> LogLevel {
    LogLevel::Info
}

fn default_env() -> String {
    "dev".into()
}
