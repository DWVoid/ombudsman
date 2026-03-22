//! ombudsman-cli: Interactive command-line client for ombudsman.
//!
//! Connects to `ombudsman-server` via WebSocket and provides a full-featured
//! interactive chat interface with all slash commands supported.

mod ws_client;

use clap::Parser;
use colored::Colorize;
use ombudsman_core::protocol::{ClientMsg, ServerMsg};
use rustyline::{error::ReadlineError, DefaultEditor};
use std::io::{self, Write};
use tracing::warn;
use ws_client::{WsClient, WsEvent};

const LOGO: &str = "🐈";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const EXIT_COMMANDS: &[&str] = &["exit", "quit", "/exit", "/quit", ":q"];

#[derive(Parser)]
#[command(
    name = "ombudsman-cli",
    version = env!("CARGO_PKG_VERSION"),
    about = "🐈 ombudsman — interactive CLI client",
)]
struct Cli {
    /// WebSocket server URL (e.g. ws://127.0.0.1:7878/ws).
    #[arg(long, default_value = "ws://127.0.0.1:7878/ws")]
    server: String,

    /// Model to request (passed as session context — the server uses its configured model).
    #[arg(short, long)]
    model: Option<String>,

    /// Session key to use (e.g. "cli:work").
    #[arg(short, long, default_value = "cli:default")]
    session: String,
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("ombudsman_cli=warn".parse().unwrap()),
        )
        .with_target(false)
        .init();

    let args = Cli::parse();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime");

    rt.block_on(run(args));
}

async fn run(args: Cli) {
    println!("{} ombudsman-cli v{}", LOGO, VERSION);
    println!("Connecting to {}...", args.server.cyan());

    // Connect to the WebSocket server.
    let mut client = match WsClient::connect(&args.server).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}", format!("Failed to connect: {}", e).red());
            eprintln!("Make sure ombudsman-server is running.");
            std::process::exit(1);
        }
    };

    println!("{}", "Connected!".green());
    if let Some(ref model) = args.model {
        println!("Requested model: {}", model.cyan());
    }
    println!("Session: {}", args.session.cyan());
    println!(
        "{}",
        "Type your message. Slash commands: /new, /stop, /status, /sessions, /help. Type 'exit' to quit."
            .dimmed()
    );
    println!();

    let history_path = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("ombudsman")
        .join("cli_history.txt");

    let mut rl = DefaultEditor::new().expect("Failed to create readline editor");
    if history_path.exists() {
        let _ = rl.load_history(&history_path);
    }

    let session_key = args.session.clone();

    loop {
        // Check for any pending server messages (non-blocking).
        // We handle them before showing the prompt to print progress etc.
        drain_pending_events(&mut client, false).await;

        let prompt = format!("{} ", "You:".bold().green());
        let line = match rl.readline(&prompt) {
            Ok(l) => l,
            Err(ReadlineError::Interrupted) => {
                println!("^C (use 'exit' to quit)");
                continue;
            }
            Err(ReadlineError::Eof) => {
                println!("Goodbye!");
                break;
            }
            Err(e) => {
                eprintln!("Readline error: {}", e);
                break;
            }
        };

        let input = line.trim().to_string();
        if input.is_empty() {
            continue;
        }
        let _ = rl.add_history_entry(&input);

        if EXIT_COMMANDS.contains(&input.to_lowercase().as_str()) {
            println!("Goodbye!");
            break;
        }

        // Handle slash commands.
        let lower = input.to_lowercase();
        let lower = lower.trim();

        if lower == "/sessions" {
            // Request session list from server.
            if let Err(e) = client.send(ClientMsg::SessionList).await {
                eprintln!("{}", format!("Send error: {}", e).red());
                continue;
            }
            // Wait for the SessionList response.
            wait_for_session_list(&mut client).await;
            continue;
        }

        if lower == "/stop" {
            if let Err(e) = client
                .send(ClientMsg::Stop {
                    session_key: session_key.clone(),
                })
                .await
            {
                eprintln!("{}", format!("Send error: {}", e).red());
            } else {
                println!("{}", "Stop signal sent.".dimmed());
                drain_pending_events(&mut client, true).await;
            }
            continue;
        }

        if lower == "/help" {
            print_help();
            continue;
        }

        // All other slash commands (/new, /status, custom) → Command message.
        // Regular text → Chat message.
        let msg = if lower.starts_with('/') {
            ClientMsg::Command {
                session_key: session_key.clone(),
                command: input.clone(),
            }
        } else {
            ClientMsg::Chat {
                session_key: session_key.clone(),
                content: input.clone(),
                media: vec![],
            }
        };

        if let Err(e) = client.send(msg).await {
            eprintln!("{}", format!("Send error: {}", e).red());
            break;
        }

        // Wait for the response, streaming progress.
        wait_for_response(&mut client, lower.starts_with('/') && lower != "/new").await;
    }

    // Save readline history.
    if let Some(parent) = history_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let _ = rl.save_history(&history_path);
}

/// Drain any immediately available events (non-blocking poll).
async fn drain_pending_events(client: &mut WsClient, blocking: bool) {
    // Use a short timeout to collect pending messages.
    let timeout = if blocking {
        tokio::time::Duration::from_secs(5)
    } else {
        tokio::time::Duration::from_millis(50)
    };
    let _ = tokio::time::timeout(timeout, async {
        while let Ok(Some(event)) = client.try_recv().await {
            handle_event(event);
        }
    })
    .await;
}

/// Wait for a Response (or Error) from the server, printing Progress along the way.
async fn wait_for_response(client: &mut WsClient, is_command: bool) {
    if !is_command {
        print!("{}", "ombudsman: ".bold().blue());
        io::stdout().flush().ok();
    }

    let timeout = tokio::time::Duration::from_secs(300);
    let _ = tokio::time::timeout(timeout, async {
        loop {
            match client.recv().await {
                Ok(event) => match event {
                    WsEvent::Message(ServerMsg::Ack { .. }) => {
                        // Acknowledged — keep waiting.
                    }
                    WsEvent::Message(ServerMsg::Progress {
                        content,
                        is_tool_hint,
                    }) => {
                        if is_tool_hint {
                            print!("\r{} {}", "⚙".dimmed(), content.dimmed());
                        } else {
                            print!("\r{}", content.dimmed());
                        }
                        io::stdout().flush().ok();
                    }
                    WsEvent::Message(ServerMsg::Response { content, .. }) => {
                        print!("\r");
                        io::stdout().flush().ok();
                        println!("{}", format!("ombudsman: {}", content).bold());
                        println!();
                        return;
                    }
                    WsEvent::Message(ServerMsg::StatusResponse { content }) => {
                        print!("\r");
                        io::stdout().flush().ok();
                        println!("{}", content.bold());
                        println!();
                        return;
                    }
                    WsEvent::Message(ServerMsg::Error { message }) => {
                        print!("\r");
                        io::stdout().flush().ok();
                        eprintln!("{}", format!("Server error: {}", message).red());
                        println!();
                        return;
                    }
                    WsEvent::Message(ServerMsg::Stopped { .. }) => {
                        print!("\r");
                        io::stdout().flush().ok();
                        println!("{}", "Task stopped.".dimmed());
                        println!();
                        return;
                    }
                    WsEvent::Message(other) => {
                        handle_event(WsEvent::Message(other));
                    }
                    WsEvent::Disconnected(reason) => {
                        eprintln!(
                            "{}",
                            format!("Disconnected from server: {}", reason).red()
                        );
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    warn!("Receive error: {}", e);
                    return;
                }
            }
        }
    })
    .await;
}

/// Wait for a SessionList response specifically.
async fn wait_for_session_list(client: &mut WsClient) {
    let timeout = tokio::time::Duration::from_secs(10);
    let _ = tokio::time::timeout(timeout, async {
        loop {
            match client.recv().await {
                Ok(WsEvent::Message(ServerMsg::SessionList { sessions })) => {
                    if sessions.is_empty() {
                        println!("{}", "No sessions found.".dimmed());
                    } else {
                        println!("{}", "Sessions:".bold());
                        for s in &sessions {
                            println!(
                                "  {} — {} messages, last updated {}",
                                s.key.cyan(),
                                s.message_count,
                                s.updated_at.dimmed()
                            );
                        }
                    }
                    println!();
                    return;
                }
                Ok(WsEvent::Message(ServerMsg::Error { message })) => {
                    eprintln!("{}", format!("Error: {}", message).red());
                    return;
                }
                Ok(WsEvent::Disconnected(reason)) => {
                    eprintln!(
                        "{}",
                        format!("Disconnected: {}", reason).red()
                    );
                    std::process::exit(1);
                }
                Ok(_) => continue, // ignore other messages while waiting
                Err(e) => {
                    warn!("Receive error: {}", e);
                    return;
                }
            }
        }
    })
    .await;
}

/// Handle a generic event (for passthrough cases).
fn handle_event(event: WsEvent) {
    match event {
        WsEvent::Message(ServerMsg::Response { content, .. }) => {
            println!("{}", format!("ombudsman: {}", content).bold());
            println!();
        }
        WsEvent::Message(ServerMsg::Progress { content, is_tool_hint }) => {
            if is_tool_hint {
                print!("\r{} {}", "⚙".dimmed(), content.dimmed());
            } else {
                print!("\r{}", content.dimmed());
            }
            io::stdout().flush().ok();
        }
        WsEvent::Message(ServerMsg::Error { message }) => {
            eprintln!("{}", format!("Server error: {}", message).red());
        }
        WsEvent::Disconnected(reason) => {
            eprintln!("{}", format!("Disconnected: {}", reason).red());
            std::process::exit(1);
        }
        _ => {}
    }
}

fn print_help() {
    println!("{}", "🐈 ombudsman-cli commands:".bold());
    println!("  {}          — Start a new conversation", "/new".cyan());
    println!("  {}         — Stop the current running task", "/stop".cyan());
    println!("  {}       — Show server status", "/status".cyan());
    println!("  {}     — List all sessions on the server", "/sessions".cyan());
    println!("  {}         — Show this help", "/help".cyan());
    println!("  {}   — Exit the CLI", "exit / quit".cyan());
    println!();
}
