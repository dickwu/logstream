mod collector;
mod config;
mod meili;
mod models;
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
        } => {
            let cfg = config::Config {
                port,
                meili_host,
                meili_key,
            };
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
