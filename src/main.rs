mod collector;
mod config;
mod meili;
mod models;
mod mcp;
mod routes;
mod subscribers;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "logstream", about = "Real-time multi-project log collector")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the log collector server
    Serve {
        /// Server port
        #[arg(short, long, default_value = "4800")]
        port: u16,

        /// Meilisearch host
        #[arg(long, env = "MEILI_HOST", default_value = "http://localhost:7700")]
        meili_host: String,

        /// Meilisearch API key
        #[arg(long, env = "MEILI_KEY", default_value = "")]
        meili_key: String,

        /// Start MCP server for AI integration
        #[arg(long, default_value = "false")]
        with_mcp: bool,
    },

    /// Initialize Meilisearch index with proper settings
    Init {
        /// Meilisearch host
        #[arg(long, env = "MEILI_HOST", default_value = "http://localhost:7700")]
        meili_host: String,

        /// Meilisearch API key
        #[arg(long, env = "MEILI_KEY", default_value = "")]
        meili_key: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Serve {
            port,
            meili_host,
            meili_key,
            with_mcp,
        } => {
            let cfg = config::Config {
                port,
                meili_host: meili_host.clone(),
                meili_key: meili_key.clone(),
                with_mcp,
            };
            
            // Start MCP server if requested
            if with_mcp {
                let mcp_cfg = mcp::McpConfig {
                    meili_host,
                    meili_key,
                };
                tokio::spawn(async move {
                    if let Err(e) = mcp::run_mcp_server(mcp_cfg).await {
                        tracing::error!("MCP server error: {}", e);
                    }
                });
                tracing::info!("MCP server will start on stdio");
            }
            
            collector::run(cfg).await?;
        }
        Commands::Init {
            meili_host,
            meili_key,
        } => {
            meili::init_index(&meili_host, &meili_key).await?;
            tracing::info!("Meilisearch index initialized");
        }
    }

    Ok(())
}
