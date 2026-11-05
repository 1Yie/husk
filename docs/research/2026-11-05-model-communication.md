# 模型通信链路诊断

日期：2026-11-05。范围：对照 Pi 检查 Husk、korei、korei-ext 的模型通信；仅诊断。

## 结论与证据边界

离线探针复现了 Gemini 工具调用签名丢失、未完成的流被提升为成功、fallback
沿用主模型参数三种行为。前两项是协议状态处理缺陷；第三项在备用 provider 不支持主模型
或参数时造成恢复失败。同模型兼容端点之间的 fallback 仍可工作。

本次没有用户现场的 provider/model、脱敏报错或输入样例，没有调用真实模型服务，
也没有验证最终 UI 展示。因此这些发现尚未与用户现场故障建立因果对应。

## 版本与职责

| 项目 | 核实 revision | 检查范围 |
| --- | --- | --- |
| Husk | `89d667a290e212579ba1cb9f6dbd83b87c0de0c1` | provider 配置、Rig 请求投影、流归一化、会话构建 |
| korei | `24d1ba428cc7ee4e3b88aa76f0ffae07035d8b72` | 消息契约、Sampler、kernel 错误消费 |
| korei-ext | `f10b8529002b6f3b2e99191149ac8d9f97ed1fa4` | MCP 工具结果进入插件管理器的边界 |

Husk 的 `Cargo.toml` 声明 Git 依赖；本次 `Cargo.lock` 固定的两个 revision
与对应本地仓库 HEAD 一致。

Pi 通过 [DeepWiki](https://deepwiki.com/earendil-works/pi) 定位，
再核对 revision `7a52c2108daa4b8320389df53b5c504f2c9da1d3` 的原始源码：

- [Google 请求投影及签名处理](https://github.com/earendil-works/pi/blob/7a52c2108daa4b8320389df53b5c504f2c9da1d3/packages/ai/src/api/google-shared.ts#L118-L128)
- [Google 工具签名回放](https://github.com/earendil-works/pi/blob/7a52c2108daa4b8320389df53b5c504f2c9da1d3/packages/ai/src/api/google-shared.ts#L268-L275)
- [OpenAI 流完成校验及错误事件](https://github.com/earendil-works/pi/blob/7a52c2108daa4b8320389df53b5c504f2c9da1d3/packages/ai/src/api/openai-completions.ts#L687-L712)

## 发现

### P1：未完成的流被报告成功

触发条件：流未产生完成事件就结束，包括空流；OpenAI-shaped 流中的部分解析错误
也可经同一路径被隐藏。

- Husk `crates/agent-llm/src/rig_bridge.rs:2480-2486` 对
  `JsonError`、`ResponseError` 只记录 warning。
- 同文件 `:2699-2705` 在 EOF 时排空缓冲，没有校验成功终止。
- korei `crates/agent-llm-core/src/sampler.rs:383-392` 在缺少 `Done` 时
  合成无 token 计数的 `Done`，返回成功。
- Sampler 对显式错误的处理和 kernel 的失败事件分支存在；
  以上路径提前丢失了触发这些分支所需的错误。

离线探针调用实际流适配器和 Sampler，空流结果为 `Ok(())`，且输出一个
`Done { prompt_tokens: None, completion_tokens: None, cached_tokens: None }`。
分别输入两类解析错误，映射结果均为零个 chunk。

Pi 的 OpenAI 适配器检查完成原因，并保留 `supportsFinishReason` 兼容条件，
失败时产生 error 事件。建议按具体协议确认成功终止，只过滤已识别的兼容噪声帧。
本次没有证明所有 HTTP 错误都会被吞掉。

### P1：Gemini 工具调用签名在往返中丢失

触发条件：Gemini 返回携带 `thoughtSignature` 的工具调用，执行工具后继续对话。

- Husk `crates/agent-llm/src/rig_bridge.rs:2403-2423` 把 Rig 工具调用转换成
  ID、名称和参数，未保留签名。
- korei `crates/agent-llm-core/src/types.rs:567` 的 `ToolCall` 未承载签名。
- Husk `rig_bridge.rs:2059-2068` 重建调用时设置 `signature: None`。
- Responses exchange 的 provider state 恢复路径不覆盖 Gemini。

探针将带签名调用依次传入真实 `MapState`、`ToolCallAssembler`、
`to_rig_messages`，重建后的调用存在而签名为 `None`。
要求签名回放的模型因此会收到缺少原始签名的后续请求；服务端实际 HTTP 响应未验证。

Pi 按 provider/model 来源判断签名是否适合回放，并保留同来源工具调用签名。
建议设计能穿过流、消息存储和请求投影的来源绑定元数据；这涉及共享契约，
实施前需确认设计。

### P2：fallback 仅切换 provider，沿用主模型和参数

触发条件：主 provider 耗尽重试预算，备用 provider 不支持主模型名称或参数。

- Husk `crates/husk-host/src/session_manager.rs:3524-3545` 构建的 fallback
  目标只包含 provider 名称和实例。
- korei `crates/agent-llm-core/src/sampler.rs:175-193` 切换 provider 后
  继续使用同一个 `SampleRequest`，`:288-294` 传递其中的模型与参数。

探针让主 provider 返回模拟 HTTP 503，备用 provider 实际收到
`model=primary-only-model, max_tokens=Some(12345)`。
该结果确认参数沿用机制；备用服务是否拒绝取决于其模型与参数支持范围。

若产品需要跨模型恢复，建议将备用目标解析为 provider、model 及其参数的完整目标；
若只支持相同模型的多端点恢复，应在配置验证中明确约束。

## korei-ext 待验证项

`crates/agent-plugin/src/mcp.rs:619-633` 对 MCP `isError:true` 记录 warning，
`crates/agent-plugin/src/manager.rs:565-569` 仍可返回 `Ok(text)`。
错误状态可能丢失，但工具文本仍传回。本次没有完成 kernel 工具状态到 UI 的全链路验证，
因此此项仅作为后续检查入口。

## 验证记录与后续

在隔离诊断副本中，仅追加三个测试，使用本次源码及锁定的 korei 依赖；
比对确认生产 `rig_bridge.rs` 的副本差异只有新增诊断代码。
三个测试由协调者复跑，结果为 `3 passed; 0 failed`，退出码 `0`。
测试断言当前异常行为，用于确认机制，修复后的验收应改为期望行为断言。

源码、配置及生产测试没有修改。后续应先取得用户现场的模型、provider、
脱敏错误和是否涉及工具调用/fallback，再选择对应修复与端到端验收范围。
