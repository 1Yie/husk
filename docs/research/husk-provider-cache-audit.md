# Husk Provider / Prompt Cache 审查与修复

> 第一阶段实现与验证已完成：两个仓库合计 **608 项测试通过，0 失败、0 忽略**。随后对等价布尔风格清理又完整复验了受影响的 413 项。这里的本地 HTTP/SSE 测试不是实际模型缓存命中的证明。
>
> 用户已授权修改同级 `/home/ichiyo/Workspace/korei`，并明确选择：**在临时 Husk 副本中使用本地路径覆盖验证，保留真实工作区的依赖来源和锁文件**。因此，发布使用仍需先发布 korei 的协议变更，再显式更新 Husk 的 Git 依赖；本次不伪造已发布 revision。

## 1. 证据范围

- Husk 基线：`71c0f03e24ba8413c5ca76be2c63fdc74c30e733`，包括开始本任务前已存在的 `config.rs` 保存修复和用户 `Cargo.lock` 改动。
- 锁定且本机同版的 korei：`c1c88342f3fd4d88ae41ecb00bfbffe20a04fc73`。
- korei-ext：`3ffeed276f254874458fc92bcd028ffa98448c0b`；只读，没有修改依赖缓存。
- rig-core：`0.42.0`，以 Cargo metadata 确认的缓存源码为准。
- Pi：[`badlogic/pi-mono@38ff99177fba21e9c57a670f4e95adcf407c99c7`](https://github.com/badlogic/pi-mono/tree/38ff99177fba21e9c57a670f4e95adcf407c99c7)。详细原始研究见 [Pi 参考笔记](pi-provider-cache-reference.md)。
- 官方说明：[OpenAI Prompt Caching](https://developers.openai.com/api/docs/guides/prompt-caching)、[Reasoning](https://developers.openai.com/api/docs/guides/reasoning)、[Responses Create](https://developers.openai.com/api/reference/resources/responses/methods/create)、[Anthropic thinking](https://platform.claude.com/docs/en/build-with-claude/adaptive-thinking)。官方页面的模型代际差异不能用旧的统一规则覆盖。

没有使用真实模型测试网关或凭据，也没有读取 harness 的模型凭据来冒充业务网关测试。尚未拿到该业务网关的实际 API 路径/两轮脱敏请求，所以本报告不把 Responses 专属差异套到所有 Chat Completions 调用上。

## 2. 架构对照

| 环节 | Husk 实际实现 | Pi 对应机制 | 结论 |
|---|---|---|---|
| 配置/选择 | `config.rs` 的 ProviderKind；`factory.rs` 构建 RigProvider；模型 compat 逐字段覆盖 provider | `Model.api` 选择 transport，`Model.compat` 由各 transport 解析 | 两者均应先按协议，再按部署/模型缩小能力；不需要重构成同一套类 |
| 能力 | korei `Capabilities` 是协议基线，`ProviderCompat` 和 `ModelParams` 是模型/部署覆盖 | API 专属 compat 解析器，默认值并非全局统一 | Husk 需要修正实际失效的覆盖，不是补齐所有 Pi 字段 |
| 请求 | `RigProvider::chat_stream` 构造 rig CompletionRequest，rig 再转成对应 API 的强类型请求 | transport 直接组装 API 参数 | Husk 的二次反序列化会丢未知参数或拒绝合法的新字段，是本次实质风险 |
| 消息 | `to_rig_messages` 转换角色、图像、工具调用/结果并修补配对 | `resolveTranscript` / `transformMessages` / API 消息转换 | 配对机制有用；Responses 不能沿用会丢失单轮身份的合并策略 |
| HTTP/SSE | rig 负责连接/SSE/标准化；SsePatchClient 负责窄范围 wire 兼容 | SDK/transport 直接处理 provider 事件 | 保留 rig，不另写整个 HTTP 栈；在现有 wire 边界补它未表达的信息 |
| 响应/历史 | 原来只留下文本 reasoning、调用 id 和 usage；内核将其写为 ChatMessage | reasoning JSON signature、text id/phase、tool item id 和 providerThinkingLevel 进入消息 | 显示用 reasoning 不能替代协议回放状态 |
| 下一轮 | 全历史无状态重发，无默认 previous_response_id | 标准 Responses 同样全历史重发且 `store:false`；Codex transport 是另一个实现 | 不应为了缓存强行加入 previous_response_id，也不能重复发送完整历史和服务器链 |

### 宿主实际链路

`SessionActor::begin_turn` → `run_engine_turn` → korei `Engine::run_rounds` → compaction/sanitize → `request_tools_schema` → `Sampler` → `LlmProvider::chat_stream` → rig HTTP/SSE → StreamChunk → Engine 历史 → SessionStore → 下一轮。

已核查的正确机制：

- `conversation = salt:session_id:canonical_workspace` 在会话内固定，模型切换、fallback 和 provider 重建透传同一身份。不能声称 Husk 每轮都会新建 Session ID。
- 内置 ToolRegistry 使用 BTreeMap；内置工具顺序稳定。
- subagent/blueprint 块在输入目录未变化时重建为同样字节；不是无条件插入当前时间。
- workspace/git/date 的初始系统头主要在会话创建时构造。
- 压缩保留系统头，但有意替换旧历史；模式、权限、工具集实际变化时出现前缀断点是正常现象。
- `input_tokens_details.cached_tokens` 已由 rig ResponsesUsage 正确解析并进入 Done；本次测试确认这一点，不把它算作修复的根因。

Pi 的 Chat Completions `buildParams:811-816` 同样仅在第一方 OpenAI或支持 long 保留时生成缓存键；Husk 保留这一保守策略。Pi 还按 provider/URL 推断长保留等默认能力，Husk 要求显式声明。这是可检查的配置差异，不是足以单独证明网关零命中的结论。

插件工具尾部来自 korei-ext `PluginManager::exported_tools`，按 HashMap 枚举，且返回前不排序。**同一个未变的 HashMap 不代表每轮必然抖动**；重建/重载和服务器工具枚举变化才是已识别的风险面。本次在 provider 统一规范化等价工具集，而非改外部插件仓库。

## 3. 六项 Pi 能力的真实语义

引用均是固定 Pi 提交中的实际读取，不按字段名猜测。

| 字段 | Pi 读取和默认 | GPT/Responses 意义 | Husk 处理 |
|---|---|---|---|
| supportsMidConvoEffort | Responses 默认 false；Anthropic 也有独立读取 | Responses 将历史 effort 记录为 configuration_update，顶层固定为 medium；不是所有端点都支持，ChatGPT sign-in 硬禁用 | 复用已存在字段，只在 Responses 显式声明支持时启用该协议路径；历史 effort 随 opaque state 持久化；未声明时保留原请求级 effort |
| supportsMidConvoSystemMessages | 默认 false；Responses、Completions、Anthropic 都有适配 | 决定后加 system 是否原位保留，而不是折到最前面；因此可能影响缓存前缀 | 没有机械新增字段。当前固定系统头和显式模式变更的行为分开说明，不为未声明端点伪造增量 system 能力 |
| supportsMidConvoToolChanges | 默认 false；实际读取在 Anthropic | Claude 的 tool_addition/tool_removal/beta 路径；不是 GPT additional_tools 的开关 | 不发送给 GPT；GPT 的增量工具另依赖 supportsAdditionalTools 和系统消息锚点。稳态工具排序无需实现整套增量工具协议 |
| forceAdaptiveThinking | Anthropic 用 `=== true`，不属于 GPT reasoning.effort | 无 GPT 请求作用 | 保留 Husk 原来的 adaptive 默认；显式 false 切换为受 max_tokens 约束的预算模式，不能继续无条件 adaptive |
| supportsTemperature | Pi 的实际读取在 Anthropic，默认 true；GPT transport 不是靠此字段统一过滤 | 不能把 Pi 的 Anthropic 默认当作所有 GPT 的能力表 | 为已复现的跨模型参数拒绝增加一个通用可选能力 supports_temperature，按模型覆盖，false 在最终 wire 边界也不能被 sampling 绕过；不按模型名字硬编码 |
| supportsStrictTools | Pi 实际读取在 Anthropic，默认 false | GPT 对应的是另一个 supportsStrictMode 和工具自身 constrained-sampling 意图 | 接受 strict 的能力不等于强制所有工具 strict。本次证实非严格默认不会发送不受支持的 strict 字段，不做盲目全局开启 |

模型适用范围由 model.compat 显式声明，不是 transport 根据 GPT/Claude 名字通配。固定提交的 Anthropic 代码注释以 Opus 4.7/Mythos 为 adaptive 示例；旧模型走预算分支。生成目录引用 `providers/data/*.json` 构建数据，但本次固定 GitHub 原始 URL 返回 404，因此不声称核对了发布包中每个模型的实际 compat 值。确定的结论是上述 transport 读取和默认值，不是全模型目录的推断。

Pi 的具体参考：

- [Responses buildParams](https://github.com/badlogic/pi-mono/blob/38ff99177fba21e9c57a670f4e95adcf407c99c7/packages/ai/src/api/openai-responses.ts#L340-L425)：store、cache key、retention/options、effort 固定基线、include。
- [configuration_update 与工具增量](https://github.com/badlogic/pi-mono/blob/38ff99177fba21e9c57a670f4e95adcf407c99c7/packages/ai/src/api/openai-responses-shared.ts#L120-L280)。
- [reasoning/text/tool 原始身份回放](https://github.com/badlogic/pi-mono/blob/38ff99177fba21e9c57a670f4e95adcf407c99c7/packages/ai/src/api/openai-responses-shared.ts#L326-L428)。
- [GPT 工具严格模式不是支持即强制](https://github.com/badlogic/pi-mono/blob/38ff99177fba21e9c57a670f4e95adcf407c99c7/packages/ai/src/api/openai-responses-shared.ts#L440-L481)。
- [Claude 独立的工具/温度/adaptive/budget 分支](https://github.com/badlogic/pi-mono/blob/38ff99177fba21e9c57a670f4e95adcf407c99c7/packages/ai/src/api/anthropic-messages.ts#L1160-L1315)。

## 4. 已证实的缺陷与最小修复

代码位置采用文件和符号，避免把外部源码缓存路径误当成可修改仓库。

| 编号 | 位置 | 触发/实际问题 | 影响与修复 |
|---|---|---|---|
| F1 | Husk `rig_bridge.rs::apply_prompt_cache_params` | 默认 short + 网关 URL 时，不发 prompt_cache_key，只有非标准亲缘头 | 标准 Responses 网关不会因此自动获得 Pi 同样的路由信息。Responses 现在发送稳定标准 key；Completions 保留保守策略；官方 host 判定不再用子串 |
| F2 | Husk `MapState::map_one`、`to_rig_messages`；korei `types.rs::ChatMessage/StreamChunk`、`engine.rs::run_rounds` | 丢掉 completed reasoning item 的 encrypted_content、item id、消息 phase 和 tool item id；重建/重载后无从回放 | 扩展为可持久化、不显示的不透明 ProviderState；由 provider 验证来源/模型/消息内容，回放原始 output 项；不使用隐藏的进程内历史缓存 |
| F3 | Husk `tool_defs`；只读 korei-ext `manager.rs::exported_tools` | 等价工具集枚举顺序变化会改变 tools 数组 | 按工具名稳定排序。测试交换同一工具集顺序，修复前能失败，修复后最终 wire 相同 |
| F4 | Husk `chat_stream` Responses 分支；rig AdditionalParameters | 把 max_output_tokens 塞进 additional_params 后清空原生 max_tokens；rig 强类型不认识此附加字段，静默丢弃 | 使用 CompletionRequest 原生 max_tokens，由 rig 正确映射 max_output_tokens；不应用 Chat Completions 的自定义 cap 字段 |
| F5 | 同上与 `responses_state.rs::RequestContext::patch` | text 只配置 verbosity 会被 rig 以 missing format 拒绝；prompt_cache_options 等新字段会被丢弃；部分 sampling 覆盖失效 | Responses 显式 overrides 留到实际 wire 边界。null 表示省略。新 cache options 替代自动生成的旧 TTL；显式同时配置两套寿命参数应本地拒绝 |
| F6 | korei `ProviderCompat` 解析/合并；Husk 温度过滤 | supportsTemperature:false 原来被忽略；仍发送 temperature；sampling 还可能绕过前置过滤 | 新增唯一必要的通用能力位，逐字段模型覆盖；所有协议门控，Responses 最终 override 后再次门控 |
| F7 | Husk Responses 请求分支 | supports_store 在 Completions 生效、在 Responses 被忽略 | 在 Responses 同样尊重 Husk 既有布尔策略。注意 **Husk 的现有行为是 bool 作为 store 值**，不把它静默重解释为 Pi 的“支持此字段就发送 store:false” |
| F8 | Husk Anthropic 请求分支 | 显式 force_adaptive_thinking:false 仍无条件发送 adaptive | 显式 false 用 enabled + budget_tokens，预算至少 1024 且小于 max_tokens；不可能的配置明确报错，绝不以关闭 thinking 换取通过；None 保留既有默认 |
| F9 | Husk Responses effort 请求和共享历史 | 现有 supports_mid_convo_effort 没有作用，且没有逐轮已解析 effort 可回放 | 仅显式声明的 Responses 路径用固定顶层 effort + append-only configuration_update。旧端点仍走原行为；显式 sampling.reasoning 覆盖不被偷偷改造成此机制 |
| F10 | Husk `to_rig_messages` / `merge_consecutive` | 为需要交替角色的协议准备的合并也无条件用于 Responses；追加相邻 user/host 内容会改写旧 input item，assistant 回合身份也会丢失 | Responses 保留消息边界；新增相邻 user 输入的前缀测试，其他协议保留既有合并行为 |
| F11 | Husk `ProviderFactory::model_params_from` | reasoning:false 模型未收敛协议基线；调用方 hint 甚至 sampling.reasoning 仍可发出控制字段 | 工厂把显式非推理模型收敛为不接受 effort；Responses 最终 wire 能力门控不被 overrides 绕过 |

实施中的交互防回归也有独立失败/转绿证据：原始 encrypted reasoning 项必须在旧方言修补之后插回，否则旧 `promote_reasoning_text` 会给真实项新增 content；显式 thinking-as-text 模式则保留旧文本方言，不被新 opaque 回放/增量 effort 覆盖。两项都已有真实 HTTP 回归测试。

### 回放的安全和正确性边界

- 回放状态带 version、模型、来源摘要和原始 output；来源包含 endpoint、鉴权材料摘要、显式头、User-Agent、鉴权模式。
- 不记录明文 API key，ProviderState 的 Debug 只给数据长度，不打印 opaque 内容。
- 只采集 output_item.done 和完成终态，不把 output_item.added 的不完整加密内容用于回放。
- 消息被 hook、脱敏、预算裁剪修改，或工具调用被移除时，必须丢弃旧 replay state，而不是恢复被清除的内容。
- 不同模型、账户或来源不回放旧的 opaque 状态。
- 对 Responses 不再合并不同 assistant 回合；其 id 和 phase 必须留在各自回合；相邻 user 消息也不能因新输入而改写旧 item。
- Engine 在成功回合落库，重试时清除失败尝试的状态。取消/错误的部分回答不冒充完成回放状态。
- 保留既有 DeepSeek Responses reasoning_text 的显式方言修复，不给普通 GPT 合成假的 reasoning id。
- 不默认发送 previous_response_id，不通过服务器状态链掩盖客户端历史丢失。

## 5. 当前兼容字段的 API 绑定

| Husk 字段 | 实际适配范围/注意 |
|---|---|
| supports_store | 两条 OpenAI wire；保留既有 bool-as-storage-policy 语义，None 省略；不是缓存开关 |
| supports_developer_role | Completions 改 system/developer；Responses 的 false 选择 input system-message fallback，默认用 instructions；不是完全相同的 wire 行为 |
| supports_reasoning_effort | 是否接受请求侧 effort 控制；模型覆盖先解析，再按协议变成 reasoning.effort、reasoning_effort 或原生 thinking 控制 |
| requires_reasoning_content_on_assistant_messages | Completions reasoning_content；显式声明的第三方 Responses thinking 方言用 reasoning_text 修复；普通 GPT 不应默认开启 |
| max_tokens_field | Chat Completions；Responses 始终用原生 max_output_tokens，Anthropic 用 max_tokens |
| send_session_affinity_headers | 控制 HTTP 头，不是 prompt_cache_key，也不是自动缓存的必要开关 |
| session_affinity_format / 自定义头名 | 仅控制网关 dialect/header 命名；会话 key 稳定已核实，不能代替对标准请求体的检查 |
| supports_temperature（新增） | 跨协议模型能力，None 保持旧行为，false 禁止最终发送，模型显式覆盖优先 |

字段接线审计现在从真实 `ProviderCompat` 定义校验完整字段清单，新增温度能力不会漏列；已实现的 force_adaptive_thinking 和 supports_mid_convo_effort 从旧 KNOWN_DEAD 移除。仍保留显式债务：supports_finish_reason、thinking_token_budget_field、supports_thinking_token_budget、supports_strict_mode、supports_openai_grammar_tools、supports_eager_tool_input_streaming、allow_empty_signature、supports_strict_tools、allowed_fallback_models。**有某处 reader 不等于每个 API 都支持该字段**，静态审计不代替协议级测试。

对于新一代 OpenAI cache options，不按模型名称推断。已知支持的部署可显式配置 sampling_params 中的 prompt_cache_options，例如只指定 ttl；旧部署继续使用既有保留参数。不要未经能力确认强开 explicit 模式：该模式还需要合适的显式断点，本地参数往返测试不证明断点有效。

## 6. 根因分级：已证实与未证实

**已证实的客户端缺陷**：F1–F11 有源码路径；请求级/快照级失败断言已复现主要失效方式。固定上下文和有效 SSE fixture 已排除测试数据格式错误。Engine 正常工具循环、下一轮采样、重载、失败重试及检查点恢复均已验证；完整回归与静态检查结果列于第 7 节。

独立审查提出的原始项被旧方言改写问题已用失败测试证实并修复。另外两项猜测没有成为缺陷：Responses 的 `wire(true, ...)` 固定启用采集包装；消息转换只查询 replay group，真正 apply 只在 wire 边界运行一次。增量 effort 测试锁定了历史/当前 marker 的预期数量。

**不能据此宣称已找到唯一的真实网关根因**：

- 缺少 prompt_cache_key 是与 Pi 的实际请求差异，但 OpenAI 官方并不说它是所有模型缓存的必要条件；较新模型的意义也不同。
- Reasoning/phase/原始 item 连续性丢失是实际协议缺陷，但不能推断每种网关都会因此让 system/tools 的所有可缓存前缀都失效。
- 工具枚举顺序风险取决于是否有插件及其重载，不能声称同一个 HashMap 每轮必变。
- 实际缓存仍取决于模型代际、可缓存的可见前缀长度、断点、TTL、组织/region、网关路由和服务端实现。官方文档当前不是“任何 GPT 都固定 1024 tokens”这一条规则。

**已排除/未当作缺陷修复**：缓存统计显示、会话内每轮随机换 key、内置工具 BTreeMap 顺序、把所有模型开成 strict、把 Claude adaptive 发送给 GPT。

### 关键实现位置

- [ProviderFactory 模型能力收敛](../../crates/agent-llm/src/factory.rs#L130)。
- [最终 wire 修补顺序](../../crates/agent-llm/src/rig_bridge.rs#L442)、[缓存路由](../../crates/agent-llm/src/rig_bridge.rs#L1023)、[请求/能力解析](../../crates/agent-llm/src/rig_bridge.rs#L1197)、[保留消息边界](../../crates/agent-llm/src/rig_bridge.rs#L1843)、[工具排序](../../crates/agent-llm/src/rig_bridge.rs#L2155)。
- [来源隔离](../../crates/agent-llm/src/responses_state.rs#L13)、[完整 SSE 事件采集](../../crates/agent-llm/src/responses_state.rs#L136)、[回放应用](../../crates/agent-llm/src/responses_state.rs#L301)、[修改后历史的失效检查](../../crates/agent-llm/src/responses_state.rs#L344)。
- korei [不透明状态类型](../../../korei/crates/agent-llm-core/src/types.rs#L170)、[可选消息字段](../../../korei/crates/agent-llm-core/src/types.rs#L247)、[流式状态事件](../../../korei/crates/agent-llm-core/src/types.rs#L650)、[Engine 生命周期/重试边界](../../../korei/crates/agent-kernel/src/engine.rs#L874)。
- [恢复草稿有效性](../../crates/husk-host/src/checkpoint.rs#L432)和[检查点状态消费](../../crates/husk-host/src/checkpoint.rs#L802)。

## 7. 验证与可重复命令

### 隔离验证（不改原锁）

```sh
HUSK_CACHE_RETENTION=short sh scripts/check-local-korei.sh ../korei \
  test --offline -p agent-llm --test responses_cache

HUSK_CACHE_RETENTION=long sh scripts/check-local-korei.sh ../korei \
  test --offline -p agent-llm --test responses_cache \
  explicit_sampling_parameters_survive_typed_conversion_and_null_omits_fields

sh scripts/check-local-korei.sh ../korei test --offline -p agent-llm -p husk-host
```

脚本复制当前 tracked/untracked 源码到临时目录，仅在那里添加 korei 路径 patch；使用 4 build jobs、4 test threads、nice 10，退出清理临时目录。第一阶段验证结束时，真实 Husk Cargo.lock 的原样校验值为：

`810d46f269d484285d7e49975113c341d6170fbe7d83e21e3bc0bdda8a3e4c9a`

### 最终验证结果

- Responses 真 HTTP/SSE 对照：15 项通过，覆盖稳定上下文、原始 item 回放/重载、effort 变化、scope/历史修改失效、typed 参数覆盖、strict 禁用默认、温度/非推理模型能力、工具排序、native cap/store、相邻 user 前缀、互斥 TTL，以及旧方言兼容。
- 跨协议 reasoning/summary：9 项通过，含四协议温度 opt-out、Claude 显式 adaptive/budget、无效预算报错与原 summary fallback。
- korei 快照状态旧格式兼容：通过。
- long 保留模式下的新旧缓存寿命替代：通过；显式同时配置互斥寿命参数：发 HTTP 前明确报错。
- Engine 工具循环落库/下一请求/重载与失败重试：2 项通过。
- 标准多行 SSE data 事件：先复现了 SDK 能解码、状态采集却丢失的问题，再按完整 SSE 事件拼接 data 修复并通过；原有单行/拆包兼容保留。
- 只在 event header 声明类型、JSON 内没有 type 的额外探针未通过当前 SDK，未把该非标准网关形状列为已支持。
- compat 字段清单/真实 schema/接线债务：1 项通过。
- 完整 Provider 试跑已通过 58 项单元测试；随后字段审计暴露临时 --config 不向 cargo metadata 子进程传递的问题，已改为只在临时副本自动发现的 .cargo/config.toml 加 patch，并修正接线债务清单。不能把该次中断当作完整套件通过。
- 检查点/真实 SessionStore 草稿恢复：11 项通过，包含普通完成草稿、只有 opaque 上下文的完成回合、失败写盘重试及保活不写空输出。
- korei `agent-llm-core` + `agent-kernel` 完整测试：**195 通过，0 失败、0 忽略**。
- Husk `agent-llm` + `husk-host` 完整测试：**413 通过，0 失败、0 忽略**。两个仓库合计 **608 项**。
- CLI/TUI `--all-targets` 编译：通过。
- 两仓库常规 Clippy：通过；仍存在既有的大枚举、类型复杂度、旧测试风格等告警，不宣称 `-D warnings` 全清。本次新增温度条件的布尔简化告警已清理并复验为 0。
- 最后等价风格清理后再次完整运行 Husk 413 项、CLI/TUI 编译、Clippy 和全部改动 Rust 文件的格式检查：均通过。
- 原始锁文件 SHA256 断言：通过，值与上方一致；真实工作区 Git 依赖来源未变。
- 预算估计继续使用可见内容估算与服务端 last_prompt_tokens 的较大值；不能从 encrypted_content 字节精确反推隐藏 token，首次恢复后的极限窗口行为仍需真实上游验证。
- None effort 保持既有“无显式指令”语义，不宣称它能在任何模型上关闭内部推理；没有为了本地缓存测试关闭任何功能。
- 完整原始状态采集/回放此次落地在 OpenAI Responses；Claude/Gemini 的既有原生签名回放没有在此宣称补齐，未把它们的专有协议直接套给 GPT。
- 无真实上游命中验证；测试中的 cached_tokens=1024 来自本地合成 SSE，仅证明解析链路。

## 8. 发布接入与后续顺序

1. 本阶段两个仓库的修改和上述验证已完成。原始 `.serena` 导航工具配置不属于业务补丁，未包含在提交中。
2. 用户最终明确要求由代理完成提交、发布及依赖接入。korei 的 6 个业务文件已提交并推送为 `7326025fcb9296fed5f926a11296bb25f4022142`；Husk 的 5 个共享 crate 已锁定到该 Git 提交，没有永久本机路径覆盖。
3. 用户后续明确要求：**本任务结束后**，单独研究并适配 Codex WSS/WebSocket。本阶段交付后已进入该后续任务，参考 [Codex WSS 协议与配置](codex-wss-reference.md)；实际 WS 帧为 JSON，不能仅把 SSE 字节搬到 WebSocket。
4. WSS 阶段用户另行授权最小依赖增补：在 `agent-llm` 锁依赖中增加已锁定 `tokio-tungstenite 0.29.0` 的一条关系，不升级已有版本。其后更新上述 5 个 korei Git revision；撤回 Cargo 顺带重排的无关 Windows 依赖引用，保留原有用户锁修改。还原本次依赖关系和 Git revision 后仍得到上述原 SHA。
5. 使用已发布 Git 依赖、无本地覆盖的 Tauri `semantic` 编译检查已通过；后续 WSS 验证及用户确认“上游未开启 WS，开启后可用”的反馈见后续报告，不与本阶段 608 项测试的原始结果混淆。
