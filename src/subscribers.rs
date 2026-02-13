use std::sync::Arc;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::models::LogEntry;

/// A subscriber watching logs in real time via WebSocket.
#[derive(Debug, Clone, Deserialize)]
pub struct SubscriberFilter {
    #[serde(default)]
    pub projects: Vec<String>,
    #[serde(default)]
    pub levels: Vec<String>,
    #[serde(default)]
    pub trace_id: Option<String>,
}

impl SubscriberFilter {
    pub fn matches(&self, entry: &LogEntry) -> bool {
        if !self.projects.is_empty() && !self.projects.contains(&entry.project) {
            return false;
        }
        let level_str = entry.level.to_string();
        if !self.levels.is_empty() && !self.levels.contains(&level_str) {
            return false;
        }
        if let Some(ref tid) = self.trace_id {
            if entry.trace_id.as_deref() != Some(tid.as_str()) {
                return false;
            }
        }
        true
    }
}

/// Message sent to a subscriber.
#[derive(Serialize)]
pub struct LogEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub data: LogEntry,
}

/// Manages all live WebSocket subscribers.
pub struct SubscriberManager {
    /// subscriber_id -> sender channel
    subs: DashMap<u64, (SubscriberFilter, mpsc::UnboundedSender<String>)>,
    next_id: std::sync::atomic::AtomicU64,
}

impl SubscriberManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            subs: DashMap::new(),
            next_id: std::sync::atomic::AtomicU64::new(1),
        })
    }

    /// Register a new subscriber. Returns (id, receiver).
    pub fn subscribe(
        &self,
        filter: SubscriberFilter,
    ) -> (u64, mpsc::UnboundedReceiver<String>) {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = mpsc::unbounded_channel();
        self.subs.insert(id, (filter, tx));
        (id, rx)
    }

    /// Remove a subscriber.
    pub fn unsubscribe(&self, id: u64) {
        self.subs.remove(&id);
    }

    /// Broadcast a log entry to all matching subscribers.
    pub fn broadcast(&self, entry: &LogEntry) {
        let payload = serde_json::to_string(&LogEvent {
            event_type: "log".into(),
            data: entry.clone(),
        })
        .unwrap_or_default();

        let mut dead: Vec<u64> = Vec::new();

        for entry_ref in self.subs.iter() {
            let (id, (filter, tx)) = entry_ref.pair();
            if filter.matches(entry) {
                if tx.send(payload.clone()).is_err() {
                    dead.push(*id);
                }
            }
        }

        // Clean up disconnected subscribers
        for id in dead {
            self.subs.remove(&id);
        }
    }

    pub fn count(&self) -> usize {
        self.subs.len()
    }
}
