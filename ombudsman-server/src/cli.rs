//! Interactive CLI session (legacy mode — no WebSocket server needed).

use colored::Colorize;
use ombudsman_core::{
    builder::build_agent_loop,
    bus::MessageBus,
    config::{load_config, paths::get_data_dir},
};
use rustyline::{error::ReadlineError, DefaultEditor};
use std::io::{self, Write};
use std::sync::Arc;

const LOGO: &str = "🐈";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const EXIT_COMMANDS: &[&str] = &["exit", "quit", "/exit", "/quit", ":q"];

/// Run an interactive CLI chat session.
pub fn run_chat(model_override: Option<String>, session_override: Option<String>) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime");

    rt.block_on(async move {
        let config = load_config().unwrap_or_default();
        let model = model_override.unwrap_or_else(|| config.agents.defaults.model.clone());

        let bus = Arc::new(MessageBus::new(64));

        let agent = match build_agent_loop(bus.clone(), &config, Some(model.clone())) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("{}", format!("Failed to initialize agent: {}", e).red());
                eprintln!("Run 'ombudsman-server onboard' to configure your API key.");
                std::process::exit(1);
            }
        };

        let session_key = session_override.unwrap_or_else(|| "cli:default".to_string());

        println!("{} ombudsman-server v{}", LOGO, VERSION);
        println!("Model: {}", model.cyan());
        println!(
            "{}",
            "Type your message. Use /new, /status, /help, or 'exit' to quit.".dimmed()
        );
        println!();

        // Background task: handle subagent result messages from the bus.
        let agent_bg = Arc::clone(&agent);
        let bus_bg = Arc::clone(&bus);
        tokio::spawn(async move {
            loop {
                let maybe_msg = tokio::time::timeout(
                    tokio::time::Duration::from_millis(200),
                    bus_bg.consume_inbound(),
                )
                .await;

                let msg = match maybe_msg {
                    Ok(Some(m)) => m,
                    Ok(None) => break,
                    Err(_) => continue,
                };

                if msg.sender_id != "subagent" {
                    continue;
                }

                if let Some(response) = agent_bg.process_message(&msg, None).await {
                    println!();
                    println!("{}", format!("ombudsman: {}", response.content).bold());
                    println!();
                    print!("{} ", "You:".bold().green());
                    io::stdout().flush().ok();
                }
            }
        });

        let history_path = get_data_dir().join("history.txt");
        let mut rl = DefaultEditor::new().expect("Failed to create readline editor");
        if history_path.exists() {
            let _ = rl.load_history(&history_path);
        }

        loop {
            let prompt = format!("{} ", "You:".bold().green());
            let line = match rl.readline(&prompt) {
                Ok(l) => l,
                Err(ReadlineError::Interrupted) => {
                    println!("^C");
                    continue;
                }
                Err(ReadlineError::Eof) => {
                    println!("Goodbye!");
                    break;
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
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

            print!("{}", "ombudsman: ".bold().blue());
            io::stdout().flush().ok();

            let agent_clone = agent.clone();
            let session_key_clone = session_key.clone();
            let on_progress: Arc<dyn Fn(String, bool) + Send + Sync> =
                Arc::new(move |content: String, tool_hint: bool| {
                    if tool_hint {
                        print!("\r{} {}", "⚙".dimmed(), content.dimmed());
                    } else {
                        print!("\r{}", content.dimmed());
                    }
                    io::stdout().flush().ok();
                });

            let response = agent_clone
                .process_direct(&input, &session_key_clone, "cli", "default", Some(on_progress))
                .await;

            print!("\r");
            io::stdout().flush().ok();

            match response {
                Some(out) => println!("{}", format!("ombudsman: {}", out.content).bold()),
                None => println!("{}", "(No response)".dimmed()),
            }
            println!();
        }

        if let Some(parent) = history_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let _ = rl.save_history(&history_path);
    });
}
