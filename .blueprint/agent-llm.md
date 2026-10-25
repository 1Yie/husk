+++
owns = ["crates/agent-llm/**"]
forbidden_deps = ["agent-kernel", "agent-ipc"]
invariants = ["密钥不以明文进入请求体或日志", "StreamChunk::Keepalive 只表示上游还活着：不携带内容，不得进入会话历史、检查点草稿或 UI 事件"]
+++

LLM provider 层：把各家协议归一为 StreamChunk。

关键词：provider, streaming, openai, anthropic, gemini, rig bridge, api key, keyring, masking, sampler, reasoning summary, keepalive

## 职责
- `rig_bridge` 是唯一的 provider 实现，按 `ProviderKind`（openai_completions / openai_responses / anthropic / gemini）分发，方言差异由 `ProviderCompat` 处理
- 配置加载、密钥解析（env:/keyring:）、出站脱敏（`EgressMasker`）、重试与续写
- 客户端标识：出站请求统一带前端在启动时声明的 `User-Agent`（`set_client_user_agent`，版本取发行二进制而非本 crate），单个 provider 可用 `userAgent` 覆盖——按客户端路由或计量的网关要求客户端自报名称，而非挂 SDK 名
- 提示词缓存：请求侧断点与 `prompt_cache_key`/粘性路由头（`HUSK_CACHE_RETENTION`），路由键由调用方传入的会话身份经 SHA-256 截断派生（`build_with_conversation`），同一会话内重建 provider 不换键；`cache_warm` 按模型 `promptCache` 寿命与价格在过期前重放上一次请求保活（`HUSK_CACHE_WARMING`）
- 会话亲缘头（`x-session-affinity` / `x-session-id` / `x-opencode-session`，方言由 `sessionAffinityFormat` 选择）：未设开关时对网关端点默认发送，对第一方厂商端点（api.openai.com / api.anthropic.com / generativelanguage.googleapis.com）默认不发；显式 `sendSessionAffinityHeaders = true` 或自报 `sessionAffinityHeaders` 则任何端点都发，显式 `false` 任何端点都不发；未知网关可用 `sessionAffinityHeaders` 自报头名（列表或逗号分隔字符串），优先于方言；与 `HUSK_CACHE_RETENTION` 解耦
- 推理摘要：思考开启且不是严格回放后端时，三条线路请求可见的推理摘要（Responses `reasoning.summary`、Anthropic `thinking.display`、Gemini `includeThoughts`）；请求在流开始前被 400/422 拒绝，就去掉摘要字段重发一次。完整推理块只取 `Text` 与 `Summary`——加密与 redacted 载荷是回放令牌，不是给用户看的文字
- 存活信号：四条线路的响应体都经同一个包装客户端，每收到一段字节记一次脉搏；上游静默超过一个周期、而这期间仍有字节到达（保活 ping、SSE 注释、不外显的事件），就向调用方送一个 `StreamChunk::Keepalive`。采样器据此重置空闲计时，不外显的长思考不会被当成卡死而整段重发；上游真的一个字节都不发时，空闲计时照旧生效

## 不做
- 不知道工具语义与会话状态

## 对外契约
- `LlmProvider` trait 与归一化的 `StreamChunk`；kernel 与前端不得见到厂商原始 JSON
- `StreamChunk::Keepalive` 不带内容、不是输出：采样器吸收它，不会转给回合循环；任何直接消费 provider 流的一方（如检查点）都不得把它当作第一块内容或写进草稿
- `ProviderFactory` 接受一个不透明的会话身份字符串（宿主给出且须含本机私有盐，本层只做哈希），未提供时按实例生成随机路由键
