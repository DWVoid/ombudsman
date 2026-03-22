//! CLI commands for ombudsman.

use crate::agent::AgentLoop;
use crate::bus::MessageBus;
use crate::config::{load_config, save_config};
use crate::config::paths::{expand_path, get_config_path, get_data_dir};
use crate::config::schema::{Config, ProviderConfig};
use crate::providers::anthropic::AnthropicProvider;
use crate::providers::azure::AzureOpenAIProvider;
use crate::providers::base::{GenerationSettings, LLMProvider};
use crate::providers::openai::OpenAIProvider;
use crate::providers::registry::{
    find_by_model, find_by_name, find_gateway, get_provider_config, resolve_model_name,
    KNOWN_PROVIDERS,
};
use clap::{Parser, Subcommand};
use colored::Colorize;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::io::{self, Write};
use std::sync::Arc;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const LOGO: &str = "🐈";

/// Exit commands recognized in interactive mode.
const EXIT_COMMANDS: &[&str] = &["exit", "quit", "/exit", "/quit", ":q"];

#[derive(Parser)]
#[command(
    name = "ombudsman",
    version = VERSION,
    about = format!("{} ombudsman - Personal AI Assistant (Rust port of nanobot)", LOGO),
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start an interactive CLI session with the agent.
    #[command(alias = "agent")]
    Chat {
        /// Model to use (overrides config).
        #[arg(short, long)]
        model: Option<String>,
        /// Session key to use (default: cli:default).
        #[arg(short, long)]
        session: Option<String>,
    },
    /// Initialize workspace and configuration.
    Onboard,
    /// Show configuration.
    Config,
    /// Show version.
    Version,
}

/// Main CLI entry point.
pub fn run_cli() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("ombudsman=info".parse().unwrap()),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Chat { model, session }) => {
            run_chat(model, session);
        }
        Some(Commands::Onboard) => {
            run_onboard();
        }
        Some(Commands::Config) => {
            show_config();
        }
        Some(Commands::Version) => {
            println!("ombudsman v{}", VERSION);
        }
        None => {
            // Default: run chat
            run_chat(None, None);
        }
    }
}

/// Run interactive chat session.
fn run_chat(model_override: Option<String>, session_override: Option<String>) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime");

    rt.block_on(async move {
        let config = load_config().unwrap_or_default();
        let workspace = expand_path(&config.agents.defaults.workspace);

        // Ensure workspace exists
        std::fs::create_dir_all(&workspace).ok();
        ensure_workspace_templates(&workspace);

        let model = model_override.unwrap_or_else(|| config.agents.defaults.model.clone());

        // Build provider
        let provider: Arc<dyn LLMProvider> = match build_provider(&model, &config) {
            Some(p) => p,
            None => {
                eprintln!("{}", "No API key found for the selected model.".red());
                eprintln!("Run 'ombudsman onboard' to configure your API key.");
                std::process::exit(1);
            }
        };

        let bus = Arc::new(MessageBus::new(64));

        let agent = match AgentLoop::new(
            bus.clone(),
            provider,
            &workspace,
            Some(model.clone()),
            config.agents.defaults.max_tool_iterations,
            config.agents.defaults.context_window_tokens,
            config.tools.web_search.clone(),
            config.tools.web_proxy.clone(),
            config.tools.exec.clone(),
            config.tools.restrict_to_workspace,
            config.channels.clone(),
            config.tools.mcp_servers.clone(),
        ) {
            Ok(a) => Arc::new(a),
            Err(e) => {
                eprintln!("Failed to initialize agent: {}", e);
                std::process::exit(1);
            }
        };

        let session_key = session_override.unwrap_or_else(|| "cli:default".to_string());

        println!("{} {} v{}", LOGO, "ombudsman".bold(), VERSION);
        println!("Model: {}", model.cyan());
        println!("Workspace: {}", workspace.display().to_string().dimmed());
        println!("{}", "Type your message. Use /new, /status, /help, or 'exit' to quit.".dimmed());
        println!();

        // Spawn a background task that processes bus messages produced by subagents.
        // Subagents announce their results by publishing an InboundMessage; the
        // background loop runs them through the agent and prints the response.
        let agent_bg = Arc::clone(&agent);
        let bus_bg = Arc::clone(&bus);
        tokio::spawn(async move {
            loop {
                // Poll with a short timeout so the task exits if the bus is dropped.
                let maybe_msg = tokio::time::timeout(
                    tokio::time::Duration::from_millis(200),
                    bus_bg.consume_inbound(),
                )
                .await;

                let msg = match maybe_msg {
                    Ok(Some(m)) => m,
                    Ok(None) => break, // channel closed
                    Err(_) => continue, // timeout — loop again
                };

                // Only process messages coming from the subagent system.
                // Interactive user messages are handled by process_direct below.
                if msg.sender_id != "subagent" {
                    continue;
                }

                if let Some(response) = agent_bg.process_message(&msg, None).await {
                    println!();
                    println!(
                        "{}",
                        format!("ombudsman: {}", response.content).bold()
                    );
                    println!();
                    print!("{} ", "You:".bold().green());
                    io::stdout().flush().ok();
                }
            }
        });

        // Set up readline editor
        let history_path = get_data_dir().join("history.txt");
        let mut rl = DefaultEditor::new().expect("Failed to create readline editor");
        if history_path.exists() {
            let _ = rl.load_history(&history_path);
        }

        loop {
            let prompt = format!("{} ", "You:".bold().green());
            let line = match rl.readline(&prompt) {
                Ok(line) => line,
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

            // Check exit commands
            if EXIT_COMMANDS.contains(&input.to_lowercase().as_str()) {
                println!("Goodbye!");
                break;
            }

            // Show typing indicator
            print!("{}", "ombudsman: ".bold().blue());
            io::stdout().flush().ok();

            // Buffer for accumulated progress output
            let _last_was_progress = false;

            let agent_clone = agent.clone();
            let _session_key_clone = session_key.clone();
            let _input_clone = input.clone();

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
                .process_direct(&input, &session_key, "cli", "default", Some(on_progress))
                .await;

            // Clear progress line
            print!("\r");
            io::stdout().flush().ok();

            match response {
                Some(out) => {
                    println!("{}", format!("ombudsman: {}", out.content).bold());
                }
                None => {
                    println!("{}", "(No response)".dimmed());
                }
            }
            println!();
        }

        // Save history
        if let Some(parent) = history_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let _ = rl.save_history(&history_path);
    });
}

/// Run onboarding wizard.
fn run_onboard() {
    println!("{} {} v{}", LOGO, "ombudsman".bold(), VERSION);
    println!("Welcome to ombudsman onboarding!\n");

    let config_path = get_config_path();
    let mut config = load_config().unwrap_or_default();

    // Ask for API key
    println!("Which LLM provider would you like to use?");
    for (i, spec) in KNOWN_PROVIDERS.iter().enumerate() {
        if spec.name != "ollama" {
            println!("  {}. {}", i + 1, spec.name);
        }
    }
    println!("  {}. ollama (local)", KNOWN_PROVIDERS.len() + 1);
    println!();

    print!("Enter provider number (1-{}, default 1): ", KNOWN_PROVIDERS.len() + 1);
    io::stdout().flush().ok();
    let mut choice = String::new();
    io::stdin().read_line(&mut choice).ok();
    let choice = choice.trim().parse::<usize>().unwrap_or(1);
    let provider_idx = choice.saturating_sub(1).min(KNOWN_PROVIDERS.len() - 1);
    let provider_name = KNOWN_PROVIDERS[provider_idx].name;

    println!();
    print!("Enter your {} API key: ", provider_name);
    io::stdout().flush().ok();
    let mut api_key = String::new();
    io::stdin().read_line(&mut api_key).ok();
    let api_key = api_key.trim().to_string();

    if !api_key.is_empty() {
        let provider_config = crate::config::schema::ProviderConfig {
            api_key: api_key.clone(),
            api_base: None,
            extra_headers: None,
        };

        match provider_name {
            "openai" => config.providers.openai = provider_config,
            "anthropic" => config.providers.anthropic = provider_config,
            "openrouter" => config.providers.openrouter = provider_config,
            "deepseek" => config.providers.deepseek = provider_config,
            "groq" => config.providers.groq = provider_config,
            "gemini" => config.providers.gemini = provider_config,
            "moonshot" => config.providers.moonshot = provider_config,
            _ => config.providers.openai = provider_config,
        }
    }

    // Ask for model
    println!();
    print!("Enter model name (default: gpt-4o): ");
    io::stdout().flush().ok();
    let mut model = String::new();
    io::stdin().read_line(&mut model).ok();
    let model = model.trim().to_string();
    if !model.is_empty() {
        config.agents.defaults.model = model;
    }

    // Save config
    if let Err(e) = save_config(&config) {
        eprintln!("Failed to save config: {}", e);
    } else {
        println!("\n✅ Configuration saved to {}", config_path.display());
        println!("Run 'ombudsman chat' to start chatting!");
    }

    // Initialize workspace
    let workspace = expand_path(&config.agents.defaults.workspace);
    std::fs::create_dir_all(&workspace).ok();
    ensure_workspace_templates(&workspace);
    println!("📁 Workspace initialized at {}", workspace.display());
}

/// Show current configuration.
fn show_config() {
    let config = load_config().unwrap_or_default();
    let config_path = get_config_path();

    println!("{} {} v{}", LOGO, "ombudsman".bold(), VERSION);
    println!("Config: {}", config_path.display());
    println!();
    println!("Model: {}", config.agents.defaults.model);
    println!("Workspace: {}", config.agents.defaults.workspace);
    println!("Max tokens: {}", config.agents.defaults.max_tokens);
    println!("Max tool iterations: {}", config.agents.defaults.max_tool_iterations);
    println!();
    println!("Providers configured:");
    let p = &config.providers;
    macro_rules! show_p {
        ($name:expr, $cfg:expr) => {
            if !$cfg.api_key.is_empty() || $cfg.api_base.is_some() {
                println!("  ✓ {}", $name);
            }
        };
    }
    show_p!("openai", p.openai);
    show_p!("anthropic", p.anthropic);
    show_p!("openrouter", p.openrouter);
    show_p!("deepseek", p.deepseek);
    show_p!("groq", p.groq);
    show_p!("gemini", p.gemini);
    show_p!("moonshot", p.moonshot);
    show_p!("minimax", p.minimax);
    show_p!("azure_openai", p.azure_openai);
    show_p!("aihubmix", p.aihubmix);
    show_p!("siliconflow", p.siliconflow);
    show_p!("volcengine", p.volcengine);
    show_p!("custom", p.custom);
    show_p!("ollama", p.ollama);
    show_p!("vllm", p.vllm);
}

// ---------------------------------------------------------------------------
// Provider construction
// ---------------------------------------------------------------------------

/// Resolve which provider spec and config to use for a given model + config.
///
/// Priority (matches the reference nanobot):
/// 1. Explicit `provider` field in `agents.defaults` (when not "auto").
/// 2. Explicit `provider/model` prefix in the model name.
/// 3. Keyword match against model name.
/// 4. Gateway auto-detection via `api_key` prefix / `api_base` URL keyword.
/// 5. Fallback to the first configured provider that has an api_key.
fn resolve_provider<'a>(
    model: &str,
    config: &'a Config,
) -> Option<(&'static str, &'a ProviderConfig, String)> {
    let providers = &config.providers;
    let forced = config.agents.defaults.provider.as_str();

    let (clean_model, _prefix_provider) = resolve_model_name(model);

    // 1. Explicit provider override
    if forced != "auto" {
        let spec = find_by_name(forced)?;
        let cfg = get_provider_config(spec.name, providers);
        let key = resolve_api_key(cfg, spec.env_key);
        // Local providers don't need a key
        if !key.is_empty() || spec.is_local {
            return Some((spec.name, cfg, clean_model));
        }
    }

    // 2 + 3. Provider-prefix or keyword match
    if let Some(spec) = find_by_model(model) {
        let cfg = get_provider_config(spec.name, providers);
        let key = resolve_api_key(cfg, spec.env_key);
        if !key.is_empty() || spec.is_local {
            return Some((spec.name, cfg, clean_model));
        }
    }

    // 4. Gateway auto-detection by api_key prefix / api_base keyword
    //    (scan all configured providers that have a key or base set)
    for spec in KNOWN_PROVIDERS {
        let cfg = get_provider_config(spec.name, providers);
        let key = resolve_api_key(cfg, spec.env_key);
        let base = cfg.api_base.as_deref().unwrap_or("");
        if find_gateway(&key, base).map(|s| s.name) == Some(spec.name) {
            if !key.is_empty() {
                return Some((spec.name, cfg, clean_model.clone()));
            }
        }
    }

    // 5. Fallback: first provider with a key
    for spec in KNOWN_PROVIDERS {
        if spec.is_local {
            continue;
        }
        let cfg = get_provider_config(spec.name, providers);
        let key = resolve_api_key(cfg, spec.env_key);
        if !key.is_empty() {
            return Some((spec.name, cfg, clean_model.clone()));
        }
    }

    None
}

/// Return the API key from `cfg`, falling back to the named env var.
fn resolve_api_key(cfg: &ProviderConfig, env_key: &str) -> String {
    if !cfg.api_key.is_empty() {
        return cfg.api_key.clone();
    }
    if !env_key.is_empty() {
        return std::env::var(env_key).unwrap_or_default();
    }
    String::new()
}

/// Build an `Arc<dyn LLMProvider>` from config and model name.
///
/// Returns `None` when no API key is available for the resolved provider.
fn build_provider(model: &str, config: &Config) -> Option<Arc<dyn LLMProvider>> {
    let (provider_name, provider_cfg, clean_model) = resolve_provider(model, config)?;

    let api_key = resolve_api_key(provider_cfg, find_by_name(provider_name)
        .map(|s| s.env_key)
        .unwrap_or(""));

    let api_base = provider_cfg
        .api_base
        .clone()
        .or_else(|| {
            find_by_name(provider_name)
                .map(|s| s.api_base.to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

    let settings = GenerationSettings {
        temperature: config.agents.defaults.temperature,
        max_tokens: config.agents.defaults.max_tokens,
        reasoning_effort: config.agents.defaults.reasoning_effort.clone(),
    };

    let extra_headers = provider_cfg.extra_headers.clone();

    let provider: Arc<dyn LLMProvider> = match provider_name {
        "anthropic" => Arc::new(AnthropicProvider::new(
            &api_key,
            Some(&api_base),
            &clean_model,
            extra_headers,
            Some(settings),
        )),
        "azure_openai" => {
            if api_base.is_empty() || api_base == "https://api.openai.com/v1" {
                // No api_base configured for Azure → fall through to OpenAI
                Arc::new(OpenAIProvider::new(
                    &api_key,
                    &api_base,
                    &clean_model,
                    extra_headers,
                    Some(settings),
                ))
            } else {
                Arc::new(AzureOpenAIProvider::new(
                    &api_key,
                    &api_base,
                    &clean_model,
                    extra_headers,
                    Some(settings),
                ))
            }
        }
        _ => Arc::new(OpenAIProvider::new(
            &api_key,
            &api_base,
            &clean_model,
            extra_headers,
            Some(settings),
        )),
    };

    Some(provider)
}

/// Ensure workspace template files exist.
fn ensure_workspace_templates(workspace: &std::path::Path) {
    // Create default template files if missing
    let templates: &[(&str, &str)] = &[
        ("AGENTS.md", "# Agent Configuration\n\nCustomize your agent behavior here.\n"),
        ("SOUL.md", "# Agent Persona\n\nDefine your agent's personality and values here.\n"),
        ("USER.md", "# User Preferences\n\nDocument your preferences and context here.\n"),
        ("TOOLS.md", "# Tool Instructions\n\nCustomize how tools should be used here.\n"),
    ];

    for (filename, default_content) in templates {
        let path = workspace.join(filename);
        if !path.exists() {
            std::fs::write(&path, default_content).ok();
        }
    }

    // Ensure memory and skills directories
    std::fs::create_dir_all(workspace.join("memory")).ok();
    std::fs::create_dir_all(workspace.join("skills")).ok();

    // Create empty memory files if needed
    let memory_file = workspace.join("memory").join("MEMORY.md");
    if !memory_file.exists() {
        std::fs::write(&memory_file, "").ok();
    }
    let history_file = workspace.join("memory").join("HISTORY.md");
    if !history_file.exists() {
        std::fs::write(&history_file, "").ok();
    }
}
