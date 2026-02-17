use clap::{Parser, Subcommand};
use futures::{SinkExt, StreamExt};
use reqwest::Client;

#[derive(Parser)]
#[command(name = "logstream", about = "Logstream CLI")]
struct Cli {
    #[arg(long, default_value = "http://localhost:4800")]
    server: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Tail logs in real-time via WebSocket
    Tail {
        /// Filter by project (comma-separated)
        #[arg(short, long)]
        project: Option<String>,

        /// Filter by level (comma-separated)
        #[arg(short, long)]
        level: Option<String>,

        /// Filter by trace ID
        #[arg(short, long)]
        trace: Option<String>,
    },

    /// Search logs
    #[command(name = "search")]
    Search {
        /// Full-text search query
        query: Option<String>,

        /// Filter by project
        #[arg(short, long)]
        project: Option<String>,

        /// Filter by level
        #[arg(short, long)]
        level: Option<String>,

        /// Filter by trace ID
        #[arg(short, long)]
        trace: Option<String>,

        /// Time range (e.g. 5m, 1h, 2d)
        #[arg(short, long)]
        since: Option<String>,

        /// Max results
        #[arg(short, long, default_value = "20")]
        limit: usize,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show project breakdown
    Projects,

    /// Show full trace timeline
    Trace {
        trace_id: String,
    },

    /// Show error summary
    Errors {
        /// Time range
        #[arg(short, long, default_value = "1h")]
        since: String,

        /// Filter by project
        #[arg(short, long)]
        project: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Tail { project, level, trace } => {
            tail(&cli.server, project, level, trace).await?;
        }
        Commands::Search {
            query,
            project,
            level,
            trace,
            since,
            limit,
            json,
        } => {
            search(&cli.server, query, project, level, trace, since, limit, json).await?;
        }
        Commands::Projects => {
            projects(&cli.server).await?;
        }
        Commands::Trace { trace_id } => {
            trace(&cli.server, trace_id).await?;
        }
        Commands::Errors { since, project } => {
            errors(&cli.server, since, project).await?;
        }
    }

    Ok(())
}

// ─── Commands ───

async fn tail(
    server: &str,
    project: Option<String>,
    level: Option<String>,
    trace: Option<String>,
) -> anyhow::Result<()> {
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    let mut url = format!("{}?mode=subscribe", server.replace("http", "ws"));
    if let Some(ref p) = project {
        url.push_str(&format!("&projects={}", p));
    }
    if let Some(ref l) = level {
        url.push_str(&format!("&levels={}", l));
    }
    if let Some(ref t) = trace {
        url.push_str(&format!("&traceId={}", t));
    }

    println!("Connecting to {}...", url);

    let (ws, _) = connect_async(&url).await?;
    let (_write, mut read) = ws.split();

    println!("Connected! Streaming logs...\n");

    while let Some(msg) = read.next().await {
        match msg? {
            Message::Text(text) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(data) = v.get("data") {
                        print_log(data);
                    } else if let Some(_connected) = v.get("type").and_then(|t| t.as_str()) {
                        println!("✓ Connected with filters: {:?}", v.get("filters"));
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    Ok(())
}

async fn search(
    server: &str,
    query: Option<String>,
    project: Option<String>,
    level: Option<String>,
    trace: Option<String>,
    since: Option<String>,
    limit: usize,
    json: bool,
) -> anyhow::Result<()> {
    let client = Client::new();
    let url = format!("{}/search", server);

    let mut params = vec![];
    if let Some(ref q) = query {
        params.push(("q", q.as_str()));
    }
    if let Some(ref p) = project {
        params.push(("project", p.as_str()));
    }
    if let Some(ref l) = level {
        params.push(("level", l.as_str()));
    }
    if let Some(ref t) = trace {
        params.push(("trace_id", t.as_str()));
    }
    if let Some(ref s) = since {
        params.push(("since", s.as_str()));
    }
    let limit_str = limit.to_string();
    params.push(("limit", limit_str.as_str()));

    let url_with_params = reqwest::Url::parse_with_params(&url, &params)?;
    let resp = client.get(url_with_params).send().await?;

    if json {
        let body: serde_json::Value = resp.json().await?;
        println!("{}", serde_json::to_string_pretty(&body)?);
    } else {
        let body: serde_json::Value = resp.json().await?;
        if let Some(facets) = body.get("facets") {
            println!("Facets: {}", serde_json::to_string(facets)?);
        }
        if let Some(hits) = body.get("hits").and_then(|h| h.as_array()) {
            println!("\n{} results:\n", hits.len());
            for hit in hits.iter().rev() {
                print_log(hit);
            }
        }
    }

    Ok(())
}

async fn projects(server: &str) -> anyhow::Result<()> {
    let client = Client::new();
    let resp = client
        .get(format!("{}/projects", server))
        .send()
        .await?;
    let body: serde_json::Value = resp.json().await?;

    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

async fn trace(server: &str, trace_id: String) -> anyhow::Result<()> {
    let client = Client::new();
    let resp = client
        .get(format!("{}/trace/{}", server, trace_id))
        .send()
        .await?;
    let body: serde_json::Value = resp.json().await?;

    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

async fn errors(server: &str, since: String, project: Option<String>) -> anyhow::Result<()> {
    let client = Client::new();
    let mut url = format!("{}/errors?since={}", server, since);
    if let Some(ref p) = project {
        url.push_str(&format!("&project={}", p));
    }

    let resp = client.get(&url).send().await?;
    let body: serde_json::Value = resp.json().await?;

    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

// ─── Helpers ───

fn print_log(log: &serde_json::Value) {
    let ts = log
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .split('T')
        .nth(1)
        .unwrap_or("")
        .get(..12)
        .unwrap_or("");

    let level = log.get("level").and_then(|v| v.as_str()).unwrap_or("info");
    let project = log.get("project").and_then(|v| v.as_str()).unwrap_or("unknown");
    let message = log.get("message").and_then(|v| v.as_str()).unwrap_or("");

    let color = match level {
        "error" | "fatal" => "\x1b[31m",
        "warn" => "\x1b[33m",
        "debug" => "\x1b[90m",
        "info" => "\x1b[36m",
        _ => "\x1b[0m",
    };

    let reset = "\x1b[0m";
    let trace = log
        .get("traceId")
        .and_then(|v| v.as_str())
        .map(|t| format!(" [{}]", &t[..8.min(t.len())]))
        .unwrap_or_default();

    println!(
        "{}{}{} {:12} {} [{}] {}",
        color, level, reset, ts, project, trace, message
    );
}
