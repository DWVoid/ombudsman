//! ombudsman-server: WebSocket agent server.
//!
//! # Subcommands
//! * `serve [--host 0.0.0.0] [--port 7878] [--model ...]` — start the WebSocket server

pub mod agent;
pub mod builder;
pub mod bus;
pub mod config;
pub mod providers;
pub mod session;
mod ws;

fn main() {
    use clap::Parser;

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("ombudsman_server=info".parse().unwrap()),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    match cli.command.unwrap_or(Commands::Serve {
        host: "127.0.0.1".to_string(),
        port: 7878,
        model: None,
    }) {
        Commands::Serve { host, port, model } => {
            ws::run_server(host, port, model);
        }
    }
}

#[derive(clap::Parser)]
#[command(
    name = "ombudsman-server",
    version = env!("CARGO_PKG_VERSION"),
    about = "🐈 ombudsman — AI assistant WebSocket server",
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Start the WebSocket agent server.
    Serve {
        /// Bind address.
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// TCP port.
        #[arg(long, default_value_t = 7878)]
        port: u16,
        /// Model to use (overrides config).
        #[arg(short, long)]
        model: Option<String>,
    },
}