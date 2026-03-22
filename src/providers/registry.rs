//! Provider registry for auto-detecting providers from model names and config.

use crate::config::schema::{ProviderConfig, ProvidersConfig};

/// Known provider specification.
pub struct ProviderSpec {
    pub name: &'static str,
    pub env_key: &'static str,
    pub api_base: &'static str,
    pub model_prefixes: &'static [&'static str],
}

/// Known providers list.
pub static KNOWN_PROVIDERS: &[ProviderSpec] = &[
    ProviderSpec {
        name: "openai",
        env_key: "OPENAI_API_KEY",
        api_base: "https://api.openai.com/v1",
        model_prefixes: &["gpt-", "o1", "o3", "o4", "text-davinci"],
    },
    ProviderSpec {
        name: "anthropic",
        env_key: "ANTHROPIC_API_KEY",
        api_base: "https://api.anthropic.com/v1",
        model_prefixes: &["claude-", "anthropic/"],
    },
    ProviderSpec {
        name: "openrouter",
        env_key: "OPENROUTER_API_KEY",
        api_base: "https://openrouter.ai/api/v1",
        model_prefixes: &[],
    },
    ProviderSpec {
        name: "deepseek",
        env_key: "DEEPSEEK_API_KEY",
        api_base: "https://api.deepseek.com/v1",
        model_prefixes: &["deepseek-", "deepseek/"],
    },
    ProviderSpec {
        name: "groq",
        env_key: "GROQ_API_KEY",
        api_base: "https://api.groq.com/openai/v1",
        model_prefixes: &["groq/"],
    },
    ProviderSpec {
        name: "gemini",
        env_key: "GEMINI_API_KEY",
        api_base: "https://generativelanguage.googleapis.com/v1beta/openai",
        model_prefixes: &["gemini-", "gemini/"],
    },
    ProviderSpec {
        name: "moonshot",
        env_key: "MOONSHOT_API_KEY",
        api_base: "https://api.moonshot.cn/v1",
        model_prefixes: &["moonshot-", "kimi-"],
    },
    ProviderSpec {
        name: "ollama",
        env_key: "",
        api_base: "http://localhost:11434/v1",
        model_prefixes: &["ollama/"],
    },
];

/// Detect provider from model name.
pub fn detect_provider_from_model(model: &str) -> Option<&'static ProviderSpec> {
    let lower = model.to_lowercase();
    for spec in KNOWN_PROVIDERS {
        if spec.model_prefixes.iter().any(|p| lower.starts_with(p)) {
            return Some(spec);
        }
    }
    None
}

/// Get the configured API key and base for a given provider name.
pub fn get_provider_config<'a>(
    name: &str,
    providers: &'a ProvidersConfig,
) -> &'a ProviderConfig {
    match name {
        "openai" => &providers.openai,
        "anthropic" => &providers.anthropic,
        "openrouter" => &providers.openrouter,
        "deepseek" => &providers.deepseek,
        "groq" => &providers.groq,
        "gemini" => &providers.gemini,
        "moonshot" => &providers.moonshot,
        "ollama" => &providers.ollama,
        "vllm" => &providers.vllm,
        "azure_openai" => &providers.azure_openai,
        _ => &providers.custom,
    }
}

/// Resolve model name: strip known prefixes for specific providers.
pub fn resolve_model_name(model: &str) -> (String, Option<String>) {
    // Handle "provider/model" format (e.g. "anthropic/claude-opus-4-5")
    if let Some(pos) = model.find('/') {
        let prefix = &model[..pos];
        let remainder = &model[pos + 1..];
        // Check if prefix matches a known provider
        for spec in KNOWN_PROVIDERS {
            if spec.name == prefix.to_lowercase() {
                return (remainder.to_string(), Some(spec.name.to_string()));
            }
        }
    }
    (model.to_string(), None)
}
