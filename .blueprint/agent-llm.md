+++
owns = ["crates/agent-llm/**"]
forbidden_deps = ["agent-kernel", "agent-ipc"]
invariants = ["密钥不以明文进入请求体或日志"]
+++
LLM provider 层：把各家协议归一为 StreamChunk。

关键词：provider, streaming, openai, anthropic, gemini, rig bridge, api key, keyring, masking, sampler

## 职责
- `rig_bridge` 是唯一的 provider 实现，按 `ProviderKind`（openai_completions / openai_responses / anthropic / gemini）分发，方言差异由 `ProviderCompat` 处理
- 配置加载、密钥解析（env:/keyring:）、出站脱敏（`EgressMasker`）、重试与续写
- 提示词缓存：请求侧断点与 `prompt_cache_key`/粘性路由头（`HUSK_CACHE_RETENTION`），`cache_warm` 按模型 `promptCache` 寿命与价格在过期前重放上一次请求保活（`HUSK_CACHE_WARMING`）

## 不做
- 不知道工具语义与会话状态

## 对外契约
- `LlmProvider` trait 与归一化的 `StreamChunk`；kernel 与前端不得见到厂商原始 JSON
