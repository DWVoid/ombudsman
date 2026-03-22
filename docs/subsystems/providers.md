# LLM Providers

**Source:** `src/providers/`

The providers subsystem abstracts over multiple LLM backends behind a single async trait.  The agent loop has no direct dependency on HTTP, API keys, or model naming conventions.  Three concrete provider implementations cover all supported backends: `OpenAIProvider`, `AnthropicProvider`, and `AzureOpenAIProvider`.

---

## Table of Contents

1. [LLMProvider Trait](#1-llmprovider-trait)
2. [OpenAI-Compatible Adapter](#2-openai-compatible-adapter)
3. [Anthropic Native Adapter](#3-anthropic-native-adapter)
4. [Azure OpenAI Adapter](#4-azure-openai-adapter)
5. [Provider Registry](#5-provider-registry)
6. [Provider Auto-Detection Flow](#6-provider-auto-detection-flow)

---

## 1. LLMProvider Trait

**File:** `src/providers/base.rs`

```rust
#[async_trait]
pub trait LLMProvider: Send + Sync {
    fn get_default_model(&self) -> &str;
    async fn chat(
        &self,
        model: &str,
        messages: &[HashMap<String, Value>],
        tools: &[Value],
        settings: &GenerationSettings,
    ) -> anyhow::Result<LLMResponse>;
}
```

### GenerationSettings

Parameters that control generation quality and length:

```rust
pub struct GenerationSettings {
    pub temperature:      Option<f64>,
    pub max_tokens:       Option<u32>,
    pub reasoning_effort: Option<String>,  // for o1/o3/o4-style reasoning models
}
```

### LLMResponse

The structured response from the LLM:

```rust
pub struct LLMResponse {
    pub content:           Option<String>,
    pub tool_calls:        Vec<ToolCallRequest>,
    pub finish_reason:     String,
    pub usage:             Option<UsageStats>,
    pub reasoning_content: Option<String>,
}
```

| Field | Description |
|---|---|
| `content` | The assistant's text reply (may be `None` if only tool calls were returned) |
| `tool_calls` | Parsed list of tool invocations requested by the model |
| `finish_reason` | `"stop"`, `"tool_calls"`, `"length"`, or provider-specific values |
| `usage` | Token counts for cost tracking |
| `reasoning_content` | Internal reasoning from thinking-mode models (stripped before storage) |

### ToolCallRequest

Represents one tool invocation from the model:

```rust
pub struct ToolCallRequest {
    pub id:        String,
    pub name:      String,
    pub arguments: HashMap<String, Value>,
}
```

### Transient Error Detection

`is_transient_error(status_code: u16) -> bool` returns `true` for:
- `429` (rate limit)
- `5xx` (server errors)

The agent loop uses this to decide whether to retry a failed LLM call.

---

## 2. OpenAI-Compatible Adapter

**File:** `src/providers/openai.rs`

A concrete `LLMProvider` implementation that speaks the OpenAI chat-completions API format. Because many providers expose an OpenAI-compatible endpoint (Deepseek, Groq, Gemini, Moonshot, MiniMax, Ollama, vLLM, OpenRouter, AiHubMix, SiliconFlow, VolcEngine, and the generic custom endpoint), one adapter covers most of them.

### Struct

```rust
pub struct OpenAIProvider {
    client:        reqwest::Client,
    api_base:      String,
    api_key:       String,
    default_model: String,
    extra_headers: HashMap<String, String>,
    settings:      GenerationSettings,
}
```

### Request Format

```
POST {api_base}/chat/completions
Content-Type: application/json
Authorization: Bearer {api_key}
{extra_headers…}

{
  "model":             "<model>",
  "messages":          […],
  "tools":             […],   // omitted when tool list is empty
  "temperature":       <f64>,
  "max_tokens":        <u32>,
  "reasoning_effort":  "<string>"  // omitted when not set
}
```

For o-series reasoning models (`o1-*`, `o3-*`, `o4-*`, `gpt-5-*`) the adapter automatically switches `max_tokens` → `max_completion_tokens` and omits `temperature`.

### Supported Backends

| Provider | API Base | Notes |
|---|---|---|
| OpenAI | `https://api.openai.com/v1` | gpt-*, o1, o3, o4, gpt-5 |
| Deepseek | `https://api.deepseek.com/v1` | deepseek-* |
| Groq | `https://api.groq.com/openai/v1` | groq/* |
| Gemini | `https://generativelanguage.googleapis.com/v1beta/openai` | gemini-* |
| Moonshot | `https://api.moonshot.ai/v1` | moonshot-*, kimi-* |
| MiniMax | `https://api.minimax.io/v1` | minimax-* |
| Ollama | `http://localhost:11434/v1` | local |
| vLLM | configurable | local / self-hosted |
| OpenRouter | `https://openrouter.ai/api/v1` | gateway; any model |
| AiHubMix | `https://aihubmix.com/v1` | gateway |
| SiliconFlow | `https://api.siliconflow.cn/v1` | gateway |
| VolcEngine | `https://ark.cn-beijing.volces.com/api/v3` | gateway |
| Custom | configurable | any OpenAI-compatible endpoint |

---

## 3. Anthropic Native Adapter

**File:** `src/providers/anthropic.rs`

A dedicated provider that calls the Anthropic Messages API at `https://api.anthropic.com/v1/messages` directly.  This is **not** the OpenAI compatibility shim; it uses Anthropic's native request/response format, enabling features that the compatibility layer does not expose:

- **Thinking mode** — `extended_thinking` blocks with configurable budget tokens.
- **Prompt caching** — `cache_control` on content blocks to reduce costs.
- **Native tool_use blocks** — direct `tool_use` / `tool_result` content types.

### Message Conversion

The adapter converts the internal OpenAI-format messages to Anthropic format before sending:

| OpenAI format | Anthropic format |
|---|---|
| `role=system` messages | Extracted and joined as the `system` field |
| `role=assistant` with `tool_calls` | Converted to `tool_use` content blocks |
| `role=tool` (tool results) | Collected and emitted as a single `role=user` message with `tool_result` blocks |
| `role=user` with `image_url` | Converted to Anthropic base64 `image` blocks |

### Thinking Mode

When `reasoning_effort` is set in `GenerationSettings`, the adapter enables extended thinking:

| `reasoning_effort` | Thinking budget |
|---|---|
| `"low"` | 2 000 tokens |
| `"medium"` | 8 000 tokens |
| `"high"` | 16 000 tokens |

### API Details

```
POST https://api.anthropic.com/v1/messages
anthropic-version: 2023-06-01
x-api-key: {api_key}
Content-Type: application/json
```

---

## 4. Azure OpenAI Adapter

**File:** `src/providers/azure.rs`

A provider for Azure-hosted OpenAI models.  Key differences from the standard OpenAI adapter:

| Aspect | Standard OpenAI | Azure OpenAI |
|---|---|---|
| URL | `{base}/chat/completions` | `{api_base}/openai/deployments/{deployment}/chat/completions?api-version=2024-10-21` |
| Auth header | `Authorization: Bearer {key}` | `api-key: {key}` |
| `max_tokens` param | `max_tokens` | `max_completion_tokens` for reasoning models |
| Temperature | Sent normally | Omitted for `gpt-5-*`, `o1-*`, `o3-*`, `o4-*` |

**Configuration requirement:** `api_base` must be set to the Azure resource URL (e.g. `https://my-resource.openai.azure.com/`) or the provider falls back to standard `OpenAIProvider`.

---

## 5. Provider Registry

**File:** `src/providers/registry.rs`

### ProviderSpec

A compile-time descriptor for a known provider:

```rust
pub struct ProviderSpec {
    pub name:                    &'static str,
    pub env_key:                 &'static str,
    pub api_base:                &'static str,
    pub keywords:                &'static [&'static str],
    pub detect_by_key_prefix:    &'static str,
    pub detect_by_base_keyword:  &'static str,
    pub is_gateway:              bool,
    pub is_local:                bool,
}
```

### KNOWN_PROVIDERS

All registered providers, in priority order (gateways → standard → local):

| Provider | `env_key` | Detection keywords / rule |
|---|---|---|
| openrouter | `OPENROUTER_API_KEY` | api_key prefix `sk-or-` |
| aihubmix | `OPENAI_API_KEY` | api_base contains `aihubmix` |
| siliconflow | `OPENAI_API_KEY` | api_base contains `siliconflow` |
| volcengine | `OPENAI_API_KEY` | api_base contains `volces` |
| anthropic | `ANTHROPIC_API_KEY` | model keywords `anthropic`, `claude` |
| openai | `OPENAI_API_KEY` | model keywords `openai`, `gpt`, `o1`, `o3`, `o4`, `text-davinci` |
| deepseek | `DEEPSEEK_API_KEY` | model keyword `deepseek` |
| gemini | `GEMINI_API_KEY` | model keyword `gemini` |
| moonshot | `MOONSHOT_API_KEY` | model keywords `moonshot`, `kimi` |
| minimax | `MINIMAX_API_KEY` | model keyword `minimax` |
| groq | `GROQ_API_KEY` | model keyword `groq` |
| azure_openai | `AZURE_OPENAI_API_KEY` | api_base contains `openai.azure.com` |
| custom | _(none)_ | fallback |
| vllm | `HOSTED_VLLM_API_KEY` | model keyword `vllm` |
| ollama | `OLLAMA_API_KEY` | api_base contains `11434` or model keyword `ollama` |

### Key Functions

```rust
find_by_name(name: &str) -> Option<&'static ProviderSpec>
```
Finds a spec by config field name (e.g. `"anthropic"`).

```rust
find_by_model(model: &str) -> Option<&'static ProviderSpec>
```
Lowercases `model`, checks for `provider/model` prefix first, then keyword matches on standard providers (skipping gateways and local providers).

```rust
find_gateway(api_key: &str, api_base: &str) -> Option<&'static ProviderSpec>
```
Detects a gateway or local provider by `api_key` prefix or `api_base` URL substring.

```rust
get_provider_config<'a>(name: &str, providers: &'a ProvidersConfig) -> &'a ProviderConfig
```
Maps a provider name to the corresponding field in `ProvidersConfig`. Falls back to `custom` if unrecognised.

```rust
resolve_model_name(model: &str) -> (String, Option<String>)
```
Strips a leading `provider/` prefix from the model name (e.g. `"anthropic/claude-opus-4-5"` → `"claude-opus-4-5"`, `Some("anthropic")`).

---

## 6. Provider Auto-Detection Flow

When `agents.defaults.provider` is `"auto"` (the default), `build_provider()` in `commands.rs` resolves the provider as follows:

```
1. resolve_model_name(model)
   → strip "provider/" prefix if present
   → use stripped name as provider hint

2. find_by_model(model)          — keyword match on model name
   find_gateway(api_key, api_base) — prefix / URL match
   → first match wins

3. resolve api_key:
   a. config field api_key (if non-empty)
   b. environment variable named by ProviderSpec.env_key

4. resolve api_base:
   a. config field api_base (if set)
   b. ProviderSpec.api_base (built-in default)

5. construct provider:
   "anthropic"   → AnthropicProvider
   "azure_openai" → AzureOpenAIProvider (if api_base is an Azure URL)
   all others    → OpenAIProvider
```

When `provider` is explicitly set (e.g. `"openrouter"`), the model keyword step is skipped and the named provider's config is used directly.

