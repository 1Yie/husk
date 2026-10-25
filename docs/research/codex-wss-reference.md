# Codex / Responses WebSocket：已核实的协议边界

本笔记区分官方 Responses 协议、固定 Pi/Codex 客户端策略和 Husk 本次实现。它不是上线实测报告：没有读取本地 Codex 凭据，没有登录或向真实上游发送模型请求。

## 1. 证据基线

- [OpenAI WebSocket mode 官方指南](https://developers.openai.com/api/docs/guides/websocket-mode)，本次直接抓取并核对正文。
- Pi 公共仓库 `badlogic/pi-mono`，固定 SHA **38ff99177fba21e9c57a670f4e95adcf407c99c7**，本地 `/tmp/husk-pi-reference-1792913212-2744089`。
- 官方 `openai/codex`，固定 SHA **970111f6cb62cc02fe36a82ea7d71b37a2889c97**，本地 `/tmp/codex-ref-wss`。
- rig-core **0.42.0**，当前实际依赖版本的 `src/providers/openai/responses_api/websocket.rs`。

Pi 的关键实现：

- [请求构造](https://github.com/badlogic/pi-mono/blob/38ff99177fba21e9c57a670f4e95adcf407c99c7/packages/ai/src/api/openai-codex-responses.ts#L536)：`model`、`store:false`、`instructions`、`input`、`include:["reasoning.encrypted_content"]`、缓存键及工具/推理字段。
- [连接内续接](https://github.com/badlogic/pi-mono/blob/38ff99177fba21e9c57a670f4e95adcf407c99c7/packages/ai/src/api/openai-codex-responses.ts#L1476)：完整上下文可证明是原上下文的扩展时才改成 `previous_response_id` + 增量输入。
- [基础认证头](https://github.com/badlogic/pi-mono/blob/38ff99177fba21e9c57a670f4e95adcf407c99c7/packages/ai/src/api/openai-codex-responses.ts#L1656)：`Authorization` 和 `chatgpt-account-id`。
- [WS 请求头](https://github.com/badlogic/pi-mono/blob/38ff99177fba21e9c57a670f4e95adcf407c99c7/packages/ai/src/api/openai-codex-responses.ts#L1701)：删除旧 beta **后重新设置** `responses_websockets=2026-02-06`；另有会话/请求标识。

官方 Codex 的握手：

- [connect_websocket](https://github.com/openai/codex/blob/970111f6cb62cc02fe36a82ea7d71b37a2889c97/codex-rs/codex-api/src/endpoint/responses_websocket.rs#L526)：使用真正 WS 握手，读取 `x-reasoning-included`、`openai-model`；可选 `turn_state` 接收器会缓存响应头 `x-codex-turn-state`。
- [JSON 序列化](https://github.com/openai/codex/blob/970111f6cb62cc02fe36a82ea7d71b37a2889c97/codex-rs/codex-api/src/endpoint/responses_websocket.rs#L1028)：直接序列化 Responses WS 请求，不是发送 SSE 文本。

## 2. Endpoint 与帧格式

| 目标 | 示例 endpoint | 注意 |
|---|---|---|
| 标准 OpenAI Responses WS | `wss://api.openai.com/v1/responses` | 标准 API key；不能把 Codex OAuth 账户规则当成所有端点的要求 |
| ChatGPT Codex | `wss://chatgpt.com/backend-api/codex/responses` | OAuth access token；需要正确的账户上下文 |
| 兼容网关 | 网关自己的 `/.../responses` | 保留配置的 host、port、路径前缀和认证头，不重写为官方域名 |

客户端发送 UTF-8 JSON 文本帧，create 参数**平铺**：

```json
{
  "type": "response.create",
  "model": "configured-model",
  "store": false,
  "instructions": "The actual stable system instructions",
  "input": [{"role":"user","content":[{"type":"input_text","text":"hello"}]}],
  "include": ["reasoning.encrypted_content"],
  "prompt_cache_key": "stable-conversation-key"
}
```

不是 `{"type":"response.create","response":{...}}`，也不是 `data: ...\n\n`。WS 不需要 HTTP/SSE 的 `stream` 字段。`generate:false` 是预热能力，不代表普通推理请求必须先预热。

`instructions` **不能因某份客户端结构体没有该字段就推断为 WS 不支持**：固定 Pi 的实际请求构造会发送它。类似地，客户端没有发送 `max_output_tokens` 不能单独证明服务端禁止该参数。Husk 不再未经证明地丢弃输出上限；若特定部署拒绝标准参数，使用已有能力/采样覆盖明确表达，并以真实服务端证据验证。

## 3. 认证和握手

| 项目 | 已核实事实 / 本次策略 |
|---|---|
| `Authorization: Bearer ...` | 标准 key、OAuth access token、网关 key 不是同一种凭据；Husk 从既有 `api_key`/`env:`/`keyring:` 配置取得，不尝试猜测或刷新凭据 |
| `chatgpt-account-id` | 固定 Pi 明确写入；ChatGPT 登录场景应由现有自定义 headers 提供。静态 API key 网关不必被强制套上 OAuth 账户校验 |
| `OpenAI-Beta` | 固定 Pi WS 默认 `responses_websockets=2026-02-06`；这是 Codex 客户端策略，不能宣称现代标准 Responses WS 都要求此 beta。Husk Codex 路径默认发送，显式自定义值优先 |
| `originator` / User-Agent | 真实标识 Husk，不冒充 `codex_cli_rs`；自定义网关头保留 |
| session headers | 同一个 provider/conversation 生成稳定标识，重建时沿用会话身份；不能拿每轮随机值替代 prompt cache key |
| `x-codex-turn-state` | 官方可选握手元数据；本次没有自动实现其 per-turn 反馈生命周期，不宣称等价于 Codex 全套优化 |

握手本身仍是 HTTP/1.1 GET Upgrade，需要验证 `101`、`Connection: Upgrade`、`Upgrade: websocket`、与请求 nonce 对应的 `Sec-WebSocket-Accept`。没有协商压缩/子协议时，不接受服务器主动附带这些选项。`permessage-deflate` 是 WS 扩展，不是 TLS 功能。

Husk 使用已有 reqwest/rustls 做 HTTP upgrade，保留代理支持，再交给 tungstenite 处理帧、客户端 masking 和控制帧；不是绕过代理直接开裸 TCP，也没有自写 RFC6455 帧解析器。

## 4. 会话与续接

官方指南当前支持**默认 lane 与 named `stream_id` lanes**：

- 同 lane FIFO，不重叠；不同 lane 可并行。
- 每连接最多 16 个在途响应、32 个 distinct named lanes。
- 连接最长 60 分钟；连接消失时，其内存 previous-response 缓存也消失。

因此“官方每连接只允许一个在途响应”不是当前通用规范。Husk 首次实现选用**默认 lane 串行子集**，未实现多路复用，不声称上游不支持并发。

`store:false` 不妨碍活动 socket 的内存续接。只有服务端持久保存过的响应，才可能在连接外按 ID 恢复。完整历史仍然必须保留在应用侧。

Husk 的保守增量条件：

1. socket、endpoint、认证/请求头作用域未变；
2. model、instructions、tools、reasoning 等 input 之外的参数完全相等；
3. 当前输入以“上次完整输入 + 上次原始 output items”为精确前缀。

满足时才发送 `previous_response_id` 与新增 items。不满足（编辑、mask、裁剪、切换模型/工具/参数、重建 provider、重连）就发送完整上下文，不能引用缓存里已经被用户修改/遮蔽的旧文本。

这不替代持久化：Husk 第一阶段的原始 encrypted reasoning、item/call ID、phase 回放仍走原有 opaque ProviderState。进程内 socket 不被持久化，重建后用完整历史重新开始。

## 5. 完成、取消、错误和回退

- 服务端 JSON 事件属于 Responses streaming event 家族。Husk 只在**内部**把它转换成现有解码器接受的事件流；真实 WS 网络帧仍是 JSON。
- Ping/Pong 是控制帧，不是模型文本。保活计数不得变成用户可见输出。
- `response.done` 只有包含可确认的终态 response body 才能归一化；不能把“socket 关了”当成功完成。
- 未完成流被消费方 drop 时，关闭/丢弃该 socket 及其续接链；不发送从 Realtime API 猜来的 cancel 帧，也不承诺上游在 TCP 关闭瞬间就停止计费。
- 只有明确的握手“不支持 WS”状态（404/405/426/501），且尚未提交 `response.create` 时，允许 sticky SSE fallback。
- 401/403、限流、TLS/IO 错误不能被伪装为“不支持 WS”。生成开始之后也不能为了隐藏错误换 SSE 重发。
- `previous_response_not_found` / `websocket_connection_limit_reached` 在任何生成事件之前出现时，可丢掉 socket/链，完整上下文恢复**一次**。出现输出或第二次拒绝后直接失败。
- 任意发送 IO 错误都可能意味着部分字节已被受理；本次传输不盲目重发。

应用内核已有自己的重试/部分答案策略。本次传输层的安全边界不应被误写成“整个应用永远不会重试”。

## 6. 本次范围与验证声明

实现和测试结果另见后续交付；编写此笔记时不得把尚未跑过的用例标为通过。

明确非目标：OAuth 浏览器登录/刷新、读取 `~/.codex/auth.json`、自动提取账户 JWT、named-lane multiplex、预热接口、zstd 请求压缩、服务器 turn-state 全生命周期、真实上游缓存命中证明。

本地 WebSocket、HTTP/SSE fixture 只能验证请求、认证头转发、生命周期与回放结构，不能证明 OAuth 授权成功、实际模型可用或真实 `cached_tokens` 命中。

## 7. 对早期草稿的更正

早期后台草稿已被本文替换。尤其纠正：

1. Pi 仓库公开，不是“非公开”。
2. Pi 删除旧 beta 后会重新设置 WS beta，不能只截取 delete 语句得出“Pi 不发”。
3. `instructions` 在 Pi 的实际 Codex 请求体内，不能列成 WS 禁止字段。
4. 账户头确实由基础鉴权函数设置，不能以未检查 auth 层作为“不存在”证据。
5. 现代官方协议支持多 lane，Husk 串行是实现范围，不是服务器固有限制。
6. HTTP/SSE fallback、预热、请求压缩等客户端策略必须与通用协议要求分开。

## 8. Husk 配置示例

显式选择新 API，现有 `openai_responses` 不自动换传输：

```toml
[providers.codex]
kind = "openai_codex_responses"
base_url = "https://chatgpt.com/backend-api/codex"
api_key = "env:CODEX_ACCESS_TOKEN"
models = ["your-configured-model-id"]

[providers.codex.headers]
chatgpt-account-id = "your-account-id"
```

`CODEX_ACCESS_TOKEN` 需要是调用者合法取得的当前 access token，不是 refresh token；本实现不负责登录或刷新。示例不会自动读取任何本地 Codex 文件。

网关可以沿用自己的静态 key 和额外请求头，不必伪造 OAuth 账户：

```toml
[providers.codex_gateway]
kind = "openai_codex_responses"
base_url = "https://gateway.example/v1"
api_key = "env:GATEWAY_API_KEY"
models = ["gateway-model-id"]
userAgent = "husk"

[providers.codex_gateway.headers]
x-gateway-tenant = "your-tenant"
```

基址应是 API 前缀，不要额外附加 `/responses`；SDK 自行补上该路径。`http(s)` 会用于相应 `ws(s)` 握手，直接配置 `ws(s)` 前缀也应走同一个升级路径。自定义 `OpenAI-Beta`、`originator` 等请求头优先于 Codex 默认值。`store=false` 是该显式 API 的 stateless 策略，不改变普通 Responses 的既有 `supports_store` 含义。

## 9. 实现与验证记录

实现位于 `crates/agent-llm/src/codex_transport.rs`，通过 `rig_bridge.rs::SsePatchClient` 接入最终 wire 修补之后、原解码器之前。配置和 UI 增加一个 API 选项，没有为传输优化扩展共享内核协议。

依赖复用已锁定 `tokio-tungstenite 0.29.0`，只新增 `agent-llm` 的直接依赖关系。用户随后明确授权提交、发布 korei 并更新 Husk 的真实 Git 锁定：5 个共享 crate 均指向 `7326025fcb9296fed5f926a11296bb25f4022142`，没有永久路径覆盖。保留原有用户锁修改，撤回 Cargo 顺带重排的 20 处无关 Windows 依赖引用；还原这 5 个 Git revision 和一条 WS 依赖关系，即得到原始 SHA256 `810d46f269d484285d7e49975113c341d6170fbe7d83e21e3bc0bdda8a3e4c9a`。

新增 `tests/codex_websocket.rs` 通过公共 ProviderFactory 和真实本地 WS server 验证传输，而非只测 JSON helper。

最终临时本地联验通过：

- WSS 协议回归 **20/20**，覆盖握手/认证头、新参数和上限、连接复用、工具增量、加密/phase 重载、历史编辑与参数改变失效、一次恢复预算、错误/断线不重发、取消、Ping/Pong、非法升级、类型别名及晚到终态。
- LLM + 宿主完整测试 **433/433**，其中已包含上述 20 项；加上未再修改的 korei 核心/内核 **195 项**，共 **628 项 Rust 测试**。
- 与用户启动参数一致的 `husk --no-default-features --features semantic` Tauri 编译检查：通过（此轮仍在临时本地 korei 覆盖下）。
- CLI/TUI 全目标编译、常规 Clippy、改动文件格式和补丁检查：通过。旧代码仍有既有告警；新 WSS 模块/测试没有新增告警。
- 两个真实负面用例先红后绿：矛盾的 `response.completed`/`in_progress` 终态、以及新 response.created 后上一轮晚到 response.done 误完成新回合。入口校验终态 status/id/output，并在整个在途期按上一 response ID 过滤尾帧。

共享协议已提交并推送为 korei `7326025fcb9296fed5f926a11296bb25f4022142`（6 个业务文件，工具配置未提交）。更新真实 Git 依赖后，无覆盖的 WSS 回归 **23/23**、常规 Clippy、`cargo check --locked --offline -p husk --no-default-features --features semantic` 均通过。新增的 3 项覆盖关闭码/原因保留、凭据脱敏、部分输出阶段与不重放约束。

提交前再次使用真实 Git 依赖、`--locked --offline` 完整验证：LLM/宿主 **436/436**（包含 WSS 23 项）、Tauri `semantic`、CLI/TUI 全目标编译、常规 Clippy、改动文件格式检查全部通过；加上未变更的 korei 195 项，共 **631 项 Rust 测试**。设置页 5 项与前端生产类型检查再次通过。最终 `Cargo.lock` SHA256 为 `71c3829884e758b8cfb35e014b35e61d3c884f3ff0d8309918c7dff1946a4c12`；已断言只比原始用户锁增加请求相关的 5 个 Git revision 更新和一条 WS 依赖关系。

前端已核实结果：设置页 `provider-compat.test.tsx` **5/5 通过**；生产 `tsc --noEmit` 通过。测试类型检查存在 `streaming-state.test.ts:356-357` 的两处 `StreamItem.content` 错误；用独立 `git archive HEAD` 副本、相同 node_modules 运行对照，当前与 HEAD 的诊断完全一致，明确属于既有问题。本次没有修改无关聊天代码，也不宣称测试项目 typecheck 全绿。

## 10. 实际关闭问题的最终反馈

用户报告 `Codex WebSocket closed before a terminal response` 后，真实网络 fixture 证实原错误丢失了关闭码和原因。诊断现保留 `code`、脱敏 `reason`、`stage`、`last_event` 与连接复用状态，不记录请求内容或认证头，也不把不明断线伪装成成功完成或自动重放。

用户最终确认：**上游没有开启 WS，开启后已可用**。这是用户环境反馈，不是本地测试证明了某个参数/鉴权问题，也不是 Prompt Cache 命中的证明。使用显式 Codex WebSocket API 前需开启上游对应能力；若网关先接受 Upgrade 再关闭，不能据此断言推理请求未被接收，仍应保留现有安全报错行为。
