# LLM Provider Layer Spec

Design goal: **the kernel and UI never see vendor JSON.** Everything behind `Arc<dyn LlmProvider>` speaks one normalized protocol; adding a provider = adding one adapter file.

## Topology

```
Agent Kernel
   │  LlmProvider::chat_stream(req) -> BoxStream<StreamChunk>
   ▼
Normalized Types (llm/types.rs)
   │  ChatMessage / ToolCall / StreamChunk
   ▼
Adapters (llm/adapters/)
   ├─ openai_compat.rs   → OpenAI, xAI, DeepSeek, Ollama(/v1), vLLM, LM Studio
   ├─ anthropic.rs       → native Messages API (v2)
   └─ mock.rs            → deterministic scripted provider for tests
   ▼
reqwest pool (keep-alive, tcp_nodelay) + eventsource-stream
```

## Module layout (`crates/agent-llm/src/`)

```text
llm/
├── types.rs          # normalized model (below)
├── provider.rs       # LlmProvider trait + BoxStream alias
├── sse.rs            # shared SSE helpers on eventsource-stream
├── sampler.rs        # SamplerActor: wraps a provider stream with resilience policy
├── factory.rs        # ProviderFactory: ProviderConfig -> Arc<dyn LlmProvider>
├── config.rs         # config.toml schema + hot-reload
├── fallback.rs       # FallbackPolicy: retry → backup provider
└── adapters/
    ├── openai_compat.rs
    ├── anthropic.rs      # stub until v2
    └── mock.rs
```

## Normalized types (`types.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role { System, User, Assistant, Tool }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,   // JSON assembled incrementally from args_delta fragments
}

#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Reasoning trace (DeepSeek-R1 reasoning_content, Claude thinking, Grok reasoning)
    ReasoningDelta(String),
    /// User-visible text delta
    ContentDelta(String),
    /// Tool-call fragment — id/name arrive once, arguments stream as JSON shards
    ToolCallDelta { index: usize, id: Option<String>, name: Option<String>, args_delta: String },
    /// Terminal chunk; carries usage when the backend reports it
    Done { prompt_tokens: Option<u32>, completion_tokens: Option<u32> },
    Error(String),
}
```

Rules:
- `arguments` fragments are **concatenated**, never parsed until `Done` or the next `index` begins — JSON arrives split across chunks.
- `Done` fires exactly once per request, even on `[DONE]` sentinel vs. clean close differences between backends.
- `Error` is a stream item, not a stream termination — SamplerActor decides retry vs. propagate.

## Provider trait (`provider.rs`)

```rust
pub type BoxStream<T> = Pin<Box<dyn Stream<Item = anyhow::Result<T>> + Send>>;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn id(&self) -> &'static str;                    // "openai_compat" | "anthropic" | "mock"
    async fn chat_stream(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: Option<serde_json::Value>,
        temperature: f32,
    ) -> anyhow::Result<BoxStream<StreamChunk>>;
}
```

`Send + Sync` + `Arc` — the kernel swaps providers by replacing the `Arc`, which is what makes hot-switching free.

## OpenAI-compatible adapter (covers ~90% of backends)

One `GenericOpenAiProvider` handles every `/v1/chat/completions` SSE dialect. Mapping rules in the `bytes_stream().eventsource()` transform:

| Wire field | → StreamChunk |
|-----------|---------------|
| `delta.reasoning_content` | `ReasoningDelta` |
| `delta.content` | `ContentDelta` |
| `delta.tool_calls[i]` | `ToolCallDelta{index, id?, name?, args_delta}` |
| `data == "[DONE]"` | `Done{None, None}` |
| `usage` block (final chunk, if present) | merged into `Done` |
| non-2xx on `send()` | `Err` before stream starts — include response body in the error |

Adapter essentials: `Client::builder().tcp_nodelay(true)`, `Authorization: Bearer <key>` header, `base_url.trim_end_matches('/')` join, body injects `tools` only when `Some`.

## Anthropic adapter (v2)

Different wire: `content_block_start/delta/stop` events, `input_json_delta` for tool args, `thinking` blocks, `x-api-key` + `anthropic-version` headers, `max_tokens` required. Map `thinking_delta`→`ReasoningDelta`, `text_delta`→`ContentDelta`, `input_json_delta.partial_json`→`args_delta`. Keep it behind the same trait — the kernel must not know it exists.

## Config & hot-switch (`config.rs`, `factory.rs`)

`~/.config/<app>/config.toml`:

```toml
active_provider = "grok"
active_model = "grok-4"

[providers.grok]
type = "openai_compat"
base_url = "https://api.x.ai/v1"
api_key = "env:XAI_API_KEY"        # env: indirection — never force plaintext secrets

[providers.ollama]
type = "openai_compat"
base_url = "http://127.0.0.1:11434/v1"
api_key = "ollama"                  # Ollama ignores it but requires the header shape

[providers.claude]
type = "anthropic"
base_url = "https://api.anthropic.com/v1"
api_key = "env:ANTHROPIC_API_KEY"

# optional degrade chain: on unrecoverable 429/5xx, next provider takes over
fallback_chain = ["ollama"]
```

- `ProviderFactory::build(&ProviderConfig)` keys on `type`; unknown type = config error at load, not at first request.
- `api_key` values starting with `env:` resolve from process env at startup; missing var → warn + mark provider unavailable (don't panic).
- Hot-swap path: UI dropdown → `Bridge::set_model(provider, model)` → kernel replaces `Arc<dyn LlmProvider>` + stores `active_model`. Applies to the **next** turn; never interrupt an in-flight stream.
- Optional `notify` watch on `config.toml` → reload providers table without restart.

## Resilience (sampler.rs wraps any provider)

SamplerActor consumes `BoxStream<StreamChunk>` and applies policy uniformly:

| Policy | Value |
|--------|-------|
| Idle timeout | no chunk for 300 s → `IdleTimeout` error |
| Doom-loop | identical repeated generation → abort + resample, `doom_max_retries = 3` |
| Retry | exponential backoff on transport errors + 429/5xx; never on 4xx |
| Fallback | after retry budget exhausted → next entry in `fallback_chain` (if configured); surface degrade event to UI as a `system` message |
| Usage | last `Done` chunk's token counts feed `SessionStats.tokens_used` |

## Testing contract

- `adapters/mock.rs`: `MockProvider` replays scripted `Vec<StreamChunk>` with configurable latency — engine, compaction, and tool-loop tests run with zero network.
- Fixture files (`tests/fixtures/sse/*.txt`) hold captured real SSE transcripts per backend; adapter tests replay them through the parser.

## Non-negotiables

- Kernel code contains **no** `reqwest` calls, no vendor field names (`choices`, `content_block`, …) — all inside adapters.
- `StreamChunk` is the only type that crosses the provider boundary.
- A new provider is accepted only if kernel/UI diff = zero lines.
