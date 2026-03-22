//! Provider registry — single source of truth for LLM provider metadata.
//!
//! Modelled after the nanobot Python registry, adapted for direct HTTP calls
//! (no LiteLLM).  All providers here use the OpenAI-compatible `chat/completions`
//! endpoint unless noted otherwise in `commands.rs`.

use crate::config::schema::{ProviderConfig, ProvidersConfig};

// ---------------------------------------------------------------------------
// ProviderSpec
// ---------------------------------------------------------------------------

/// Metadata for one LLM provider.  Order in `KNOWN_PROVIDERS` controls
/// keyword-match priority — gateways first, then standard providers, then local.
pub struct ProviderSpec {
    /// Config field name, e.g. `"openai"`.
    pub name: &'static str,
    /// Environment variable that holds the API key.
    pub env_key: &'static str,
    /// Default base URL.  Empty = use the OpenAI default.
    pub api_base: &'static str,
    /// Model-name keywords (lowercase) that map to this provider.
    pub keywords: &'static [&'static str],
    /// If non-empty, detect this provider when `api_key.starts_with(prefix)`.
    pub detect_by_key_prefix: &'static str,
    /// If non-empty, detect this provider when `api_base` URL contains this substring.
    pub detect_by_base_keyword: &'static str,
    /// True for gateways that can route any model (OpenRouter, AiHubMix, …).
    pub is_gateway: bool,
    /// True for local deployments (Ollama, vLLM).
    pub is_local: bool,
}

/// All known providers.  Order matters: gateways first, then standard, then local.
pub static KNOWN_PROVIDERS: &[ProviderSpec] = &[
    // === Gateways ==========================================================
    ProviderSpec {
        name: "openrouter",
        env_key: "OPENROUTER_API_KEY",
        api_base: "https://openrouter.ai/api/v1",
        keywords: &["openrouter"],
        detect_by_key_prefix: "sk-or-",
        detect_by_base_keyword: "openrouter",
        is_gateway: true,
        is_local: false,
    },
    ProviderSpec {
        name: "aihubmix",
        env_key: "OPENAI_API_KEY",
        api_base: "https://aihubmix.com/v1",
        keywords: &["aihubmix"],
        detect_by_key_prefix: "",
        detect_by_base_keyword: "aihubmix",
        is_gateway: true,
        is_local: false,
    },
    ProviderSpec {
        name: "siliconflow",
        env_key: "OPENAI_API_KEY",
        api_base: "https://api.siliconflow.cn/v1",
        keywords: &["siliconflow"],
        detect_by_key_prefix: "",
        detect_by_base_keyword: "siliconflow",
        is_gateway: true,
        is_local: false,
    },
    ProviderSpec {
        name: "volcengine",
        env_key: "OPENAI_API_KEY",
        api_base: "https://ark.cn-beijing.volces.com/api/v3",
        keywords: &["volcengine", "volces", "ark"],
        detect_by_key_prefix: "",
        detect_by_base_keyword: "volces",
        is_gateway: true,
        is_local: false,
    },
    // === Standard providers ================================================
    ProviderSpec {
        name: "anthropic",
        env_key: "ANTHROPIC_API_KEY",
        api_base: "https://api.anthropic.com/v1",
        keywords: &["anthropic", "claude"],
        detect_by_key_prefix: "",
        detect_by_base_keyword: "",
        is_gateway: false,
        is_local: false,
    },
    ProviderSpec {
        name: "openai",
        env_key: "OPENAI_API_KEY",
        api_base: "https://api.openai.com/v1",
        keywords: &["openai", "gpt", "o1", "o3", "o4", "text-davinci"],
        detect_by_key_prefix: "",
        detect_by_base_keyword: "",
        is_gateway: false,
        is_local: false,
    },
    ProviderSpec {
        name: "deepseek",
        env_key: "DEEPSEEK_API_KEY",
        api_base: "https://api.deepseek.com/v1",
        keywords: &["deepseek"],
        detect_by_key_prefix: "",
        detect_by_base_keyword: "",
        is_gateway: false,
        is_local: false,
    },
    ProviderSpec {
        name: "gemini",
        env_key: "GEMINI_API_KEY",
        api_base: "https://generativelanguage.googleapis.com/v1beta/openai",
        keywords: &["gemini"],
        detect_by_key_prefix: "",
        detect_by_base_keyword: "",
        is_gateway: false,
        is_local: false,
    },
    ProviderSpec {
        name: "moonshot",
        env_key: "MOONSHOT_API_KEY",
        api_base: "https://api.moonshot.ai/v1",
        keywords: &["moonshot", "kimi"],
        detect_by_key_prefix: "",
        detect_by_base_keyword: "",
        is_gateway: false,
        is_local: false,
    },
    ProviderSpec {
        name: "minimax",
        env_key: "MINIMAX_API_KEY",
        api_base: "https://api.minimax.io/v1",
        keywords: &["minimax"],
        detect_by_key_prefix: "",
        detect_by_base_keyword: "",
        is_gateway: false,
        is_local: false,
    },
    ProviderSpec {
        name: "groq",
        env_key: "GROQ_API_KEY",
        api_base: "https://api.groq.com/openai/v1",
        keywords: &["groq"],
        detect_by_key_prefix: "",
        detect_by_base_keyword: "",
        is_gateway: false,
        is_local: false,
    },
    // === Azure OpenAI (direct, special auth) ================================
    ProviderSpec {
        name: "azure_openai",
        env_key: "AZURE_OPENAI_API_KEY",
        api_base: "",  // user must configure api_base
        keywords: &["azure", "azure-openai"],
        detect_by_key_prefix: "",
        detect_by_base_keyword: "openai.azure.com",
        is_gateway: false,
        is_local: false,
    },
    // === Custom (any OpenAI-compatible endpoint) ============================
    ProviderSpec {
        name: "custom",
        env_key: "",
        api_base: "http://localhost:8000/v1",
        keywords: &[],
        detect_by_key_prefix: "",
        detect_by_base_keyword: "",
        is_gateway: false,
        is_local: false,
    },
    // === Local deployment ===================================================
    ProviderSpec {
        name: "vllm",
        env_key: "HOSTED_VLLM_API_KEY",
        api_base: "",  // user must configure api_base
        keywords: &["vllm"],
        detect_by_key_prefix: "",
        detect_by_base_keyword: "",
        is_gateway: false,
        is_local: true,
    },
    ProviderSpec {
        name: "ollama",
        env_key: "OLLAMA_API_KEY",
        api_base: "http://localhost:11434/v1",
        keywords: &["ollama"],
        detect_by_key_prefix: "",
        detect_by_base_keyword: "11434",
        is_gateway: false,
        is_local: true,
    },
];

// ---------------------------------------------------------------------------
// Lookup helpers
// ---------------------------------------------------------------------------

/// Find a provider spec by config field name.
pub fn find_by_name(name: &str) -> Option<&'static ProviderSpec> {
    KNOWN_PROVIDERS.iter().find(|s| s.name == name)
}

/// Match a provider by model-name keyword (case-insensitive, normalises `-` → `_`).
///
/// Skips gateways and local providers — those are matched by api_key / api_base.
pub fn find_by_model(model: &str) -> Option<&'static ProviderSpec> {
    let model_lower = model.to_lowercase();
    let model_norm = model_lower.replace('-', "_");
    // Explicit "provider/model" prefix wins
    if let Some(slash) = model_lower.find('/') {
        let prefix = &model_lower[..slash];
        let prefix_norm = prefix.replace('-', "_");
        for spec in KNOWN_PROVIDERS {
            if spec.name == prefix || spec.name == prefix_norm {
                return Some(spec);
            }
        }
    }
    // Keyword match (standard providers only)
    for spec in KNOWN_PROVIDERS.iter().filter(|s| !s.is_gateway && !s.is_local) {
        if spec
            .keywords
            .iter()
            .any(|kw| model_lower.contains(kw) || model_norm.contains(&kw.replace('-', "_")))
        {
            return Some(spec);
        }
    }
    None
}

/// Detect a gateway or local provider from `api_key` / `api_base`.
///
/// Priority:
/// 1. `detect_by_key_prefix` match.
/// 2. `detect_by_base_keyword` match.
pub fn find_gateway(api_key: &str, api_base: &str) -> Option<&'static ProviderSpec> {
    for spec in KNOWN_PROVIDERS {
        if !spec.detect_by_key_prefix.is_empty()
            && api_key.starts_with(spec.detect_by_key_prefix)
        {
            return Some(spec);
        }
        if !spec.detect_by_base_keyword.is_empty()
            && api_base.contains(spec.detect_by_base_keyword)
        {
            return Some(spec);
        }
    }
    None
}

/// Legacy alias kept for backwards compatibility.
#[inline]
pub fn detect_provider_from_model(model: &str) -> Option<&'static ProviderSpec> {
    find_by_model(model)
}

/// Get the `ProviderConfig` for a given provider name from the config.
pub fn get_provider_config<'a>(name: &str, providers: &'a ProvidersConfig) -> &'a ProviderConfig {
    match name {
        "openai" => &providers.openai,
        "anthropic" => &providers.anthropic,
        "openrouter" => &providers.openrouter,
        "deepseek" => &providers.deepseek,
        "groq" => &providers.groq,
        "gemini" => &providers.gemini,
        "moonshot" => &providers.moonshot,
        "minimax" => &providers.minimax,
        "ollama" => &providers.ollama,
        "vllm" => &providers.vllm,
        "azure_openai" => &providers.azure_openai,
        "aihubmix" => &providers.aihubmix,
        "siliconflow" => &providers.siliconflow,
        "volcengine" => &providers.volcengine,
        "custom" => &providers.custom,
        _ => &providers.custom,
    }
}

/// Strip a leading `provider/` prefix from the model name when the provider
/// prefix is known, e.g. `"anthropic/claude-opus-4-5"` → `"claude-opus-4-5"`.
///
/// Returns `(clean_model, Option<provider_name>)`.
pub fn resolve_model_name(model: &str) -> (String, Option<String>) {
    if let Some(slash) = model.find('/') {
        let prefix = &model[..slash];
        for spec in KNOWN_PROVIDERS {
            if spec.name == prefix.to_lowercase() {
                return (model[slash + 1..].to_string(), Some(spec.name.to_string()));
            }
        }
    }
    (model.to_string(), None)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_by_model_openai() {
        let spec = find_by_model("gpt-4o").unwrap();
        assert_eq!(spec.name, "openai");
    }

    #[test]
    fn test_find_by_model_anthropic() {
        let spec = find_by_model("claude-opus-4-5").unwrap();
        assert_eq!(spec.name, "anthropic");
    }

    #[test]
    fn test_find_by_model_prefix() {
        let spec = find_by_model("anthropic/claude-opus-4-5").unwrap();
        assert_eq!(spec.name, "anthropic");
    }

    #[test]
    fn test_find_by_model_deepseek() {
        let spec = find_by_model("deepseek-chat").unwrap();
        assert_eq!(spec.name, "deepseek");
    }

    #[test]
    fn test_find_by_model_gemini() {
        let spec = find_by_model("gemini-2.5-pro").unwrap();
        assert_eq!(spec.name, "gemini");
    }

    #[test]
    fn test_find_gateway_by_key_prefix() {
        let spec = find_gateway("sk-or-v1-abc123", "").unwrap();
        assert_eq!(spec.name, "openrouter");
    }

    #[test]
    fn test_find_gateway_by_base_keyword() {
        let spec = find_gateway("", "https://my-resource.openai.azure.com/").unwrap();
        assert_eq!(spec.name, "azure_openai");
    }

    #[test]
    fn test_find_gateway_ollama() {
        let spec = find_gateway("", "http://localhost:11434/v1").unwrap();
        assert_eq!(spec.name, "ollama");
    }

    #[test]
    fn test_resolve_model_name_strips_prefix() {
        let (model, provider) = resolve_model_name("anthropic/claude-opus-4-5");
        assert_eq!(model, "claude-opus-4-5");
        assert_eq!(provider.as_deref(), Some("anthropic"));
    }

    #[test]
    fn test_resolve_model_name_no_prefix() {
        let (model, provider) = resolve_model_name("gpt-4o");
        assert_eq!(model, "gpt-4o");
        assert!(provider.is_none());
    }
}

