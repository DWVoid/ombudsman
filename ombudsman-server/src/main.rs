//! ombudsman-server: WebSocket agent server + legacy CLI.
//!
//! # Subcommands
//! * `serve [--host 0.0.0.0] [--port 7878] [--model ...]` — start the WebSocket server
//! * `chat  [--model ...]  [--session ...]`               — interactive CLI (legacy mode)

mod cli;
mod ws;

fn main() {
    use clap::Parser;

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("ombudsman_server=info".parse().unwrap())
                .add_directive("ombudsman_core=info".parse().unwrap()),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    match cli.command.unwrap_or(Commands::Chat {
        model: None,
        session: None,
    }) {
        Commands::Serve { host, port, model } => {
            ws::run_server(host, port, model);
        }
        Commands::Chat { model, session } => {
            cli::run_chat(model, session);
        }
    }
}

#[derive(clap::Parser)]
#[command(
    name = "ombudsman-server",
    version = env!("CARGO_PKG_VERSION"),
    about = "🐈 ombudsman — AI assistant server",
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
    /// Interactive CLI session (legacy mode, no server needed).
    #[command(alias = "agent")]
    Chat {
        /// Model to use (overrides config).
        #[arg(short, long)]
        model: Option<String>,
        /// Session key.
        #[arg(short, long)]
        session: Option<String>,
    },
}
