//! Convenience builder helpers for constructing the agent stack from configuration.
//!
//! These functions are used by both `ombudsman-server` (WS + CLI modes) to avoid
//! duplicating provider-resolution and workspace-setup logic.

use crate::agent::AgentLoop;
use crate::bus::MessageBus;
use crate::config::schema::{Config, ProviderConfig};
use crate::providers::anthropic::AnthropicProvider;
use crate::providers::azure::AzureOpenAIProvider;
use crate::providers::base::{GenerationSettings, LLMProvider};
use crate::providers::openai::OpenAIProvider;
use crate::providers::registry::{
    find_by_model, find_by_name, find_gateway, get_provider_config, resolve_model_name,
    KNOWN_PROVIDERS,
};
use std::path::Path;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Provider construction
// ---------------------------------------------------------------------------

/// Priority order (matches the reference nanobot):
/// 1. Explicit `provider` field in `agents.defaults` (when not "auto").
/// 2. Explicit `provider/model` prefix in the model name.
/// 3. Keyword match against model name.
/// 4. Gateway auto-detection via `api_key` prefix / `api_base` URL keyword.
/// 5. Fallback to the first configured provider that has an api_key.
pub fn resolve_provider<'a>(
    model: &str,
    config: &'a Config,
) -> Option<(&'static str, &'a ProviderConfig, String)> {
    let providers = &config.providers;
    let forced = config.agents.defaults.provider.as_str();
    let (clean_model, _prefix_provider) = resolve_model_name(model);

    if forced != "auto" {
        let spec = find_by_name(forced)?;
        let cfg = get_provider_config(spec.name, providers);
        let key = resolve_api_key(cfg, spec.env_key);
        if !key.is_empty() || spec.is_local {
            return Some((spec.name, cfg, clean_model));
        }
    }

    if let Some(spec) = find_by_model(model) {
        let cfg = get_provider_config(spec.name, providers);
        let key = resolve_api_key(cfg, spec.env_key);
        if !key.is_empty() || spec.is_local {
            return Some((spec.name, cfg, clean_model));
        }
    }

    for spec in KNOWN_PROVIDERS {
        let cfg = get_provider_config(spec.name, providers);
        let key = resolve_api_key(cfg, spec.env_key);
        let base = cfg.api_base.as_deref().unwrap_or("");
        if find_gateway(&key, base).map(|s| s.name) == Some(spec.name) && !key.is_empty() {
            return Some((spec.name, cfg, clean_model.clone()));
        }
    }

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
pub fn resolve_api_key(cfg: &ProviderConfig, env_key: &str) -> String {
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
pub fn build_provider(model: &str, config: &Config) -> Option<Arc<dyn LLMProvider>> {
    let (provider_name, provider_cfg, clean_model) = resolve_provider(model, config)?;

    let api_key = resolve_api_key(
        provider_cfg,
        find_by_name(provider_name).map(|s| s.env_key).unwrap_or(""),
    );

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

/// Build a fully-configured [`AgentLoop`] from the application config.
///
/// # Errors
/// Returns an error if `SessionManager::new` fails (e.g. bad workspace path).
pub fn build_agent_loop(
    bus: Arc<MessageBus>,
    config: &Config,
    model_override: Option<String>,
) -> anyhow::Result<Arc<AgentLoop>> {
    let model = model_override.unwrap_or_else(|| config.agents.defaults.model.clone());
    let workspace = crate::config::paths::expand_path(&config.agents.defaults.workspace);
    std::fs::create_dir_all(&workspace)?;
    ensure_workspace_templates(&workspace);

    let provider = build_provider(&model, config)
        .ok_or_else(|| anyhow::anyhow!("No API key configured for model '{}'", model))?;

    let agent = AgentLoop::new(
        bus,
        provider,
        &workspace,
        Some(model),
        config.agents.defaults.max_tool_iterations,
        config.agents.defaults.context_window_tokens,
        config.tools.web_search.clone(),
        config.tools.web_proxy.clone(),
        config.tools.exec.clone(),
        config.tools.restrict_to_workspace,
        config.channels.clone(),
        config.tools.mcp_servers.clone(),
    )?;

    Ok(Arc::new(agent))
}

// ---------------------------------------------------------------------------
// Workspace helpers
// ---------------------------------------------------------------------------

/// Create default template files and directories in the workspace.
pub fn ensure_workspace_templates(workspace: &Path) {
    let templates: &[(&str, &str)] = &[
        (
            "AGENTS.md",
            "# Agent Configuration\n\nCustomize your agent behavior here.\n",
        ),
        (
            "SOUL.md",
            "# Agent Persona\n\nDefine your agent's personality and values here.\n",
        ),
        (
            "USER.md",
            "# User Preferences\n\nDocument your preferences and context here.\n",
        ),
        (
            "TOOLS.md",
            "# Tool Instructions\n\nCustomize how tools should be used here.\n",
        ),
    ];

    for (filename, default_content) in templates {
        let path = workspace.join(filename);
        if !path.exists() {
            std::fs::write(&path, default_content).ok();
        }
    }

    std::fs::create_dir_all(workspace.join("memory")).ok();
    std::fs::create_dir_all(workspace.join("skills")).ok();

    let memory_file = workspace.join("memory").join("MEMORY.md");
    if !memory_file.exists() {
        std::fs::write(&memory_file, "").ok();
    }
    let history_file = workspace.join("memory").join("HISTORY.md");
    if !history_file.exists() {
        std::fs::write(&history_file, "").ok();
    }
}
