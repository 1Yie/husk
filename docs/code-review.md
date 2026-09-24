# agent-rs 代码评审报告

> 原始评审范围：全部 8 个 crate，~12 900 行 Rust。评审维度：缺陷（正确性/安全/并发）与通用性（抽象/耦合/可扩展性）。
>
> **阅读方式**：下面的「核对状态」是逐条在代码中重新核对后的结论（每条给出提交/测试证据），它确认或推翻正文的对应论断；「评审正文（原始快照）」保留原文以便追溯，其中的统计数字与结论已过时。

## 核对状态（最新核对：本轮修复之后）

核对方式：逐条 grep/单测定位实际代码，不采信文档推断。证据以**测试名与文件/行号**为准——提交哈希会随 amend/rebase 失效，因此不再引用；本轮的修复一次提交进入仓库。

### 已修复

| 原编号 | 问题 | 结论与证据 |
|---|---|---|
| C1 | memory `next_id` 重排、`upsert_fact` 跨事务 dedupe | **已修复**：id 计数行进 META，并在**同一个 write txn 内**读取+自增+插入；`upsert_fact` 的扫描/判定/写入合并为单 txn（原先「读事务扫描 + 写事务插入」可双双过 0.92 dedupe 产生重复 fact）；`open` 对旧库按 max(id)+1 播种。测试 `counter_row_is_seeded_and_advances`、`reopened_store_continues_above_everything_written`、`open_seeds_a_missing_counter_row`。附注：redb 默认 `ExclusiveWriter` 本就拒绝第二个活动实例（第二个 session 的 `open` 失败 → `(no memory)`），故「跨实例重叠」实际只在重开或多写者模式下可达，两者都已被计数行覆盖 |
| C2 | undo 不查外部修改，覆盖用户编辑 | **已修复**：新增 `HunkTracker::mark_external_if_drifted`（磁盘字节 vs 该文件最后一个 hunk 的 `new`，不一致即走 `handle_external_change` 置标志），`undo_last_turn` 在构造 undo plan 前调用；不可读文件与被逐出的 hunk 不判定。测试 `drifted_file_refuses_undo`、`matching_or_unreadable_files_are_not_flagged`、`drift_compares_against_the_newest_write`。根因（无 watcher → 标志恒为 false）随 `AmbientProbe` 删除一并了结 |
| C3 | 二进制/非 UTF-8 写入无法 undo | **已修复**：`Hunk.old/new: Vec<u8>`、`record_write` 收字节、`UndoOp.restore_to: Option<Vec<u8>>`；测试 `binary_writes_are_recorded` |
| C4 | `Cancel` 空操作 | **已修复**：`run_turn` 每个 sample/tool 轮顶部检查 `io.cancel`（`CANCEL_ERR`/`CANCEL_TEXT`）；前端 `commands.ts` 确实发送 `Cancel`；沙箱超时改为后端 tree-kill |
| C5（后半） | masker 不 scrub `tool_calls[].arguments`；PEM 只遮首行 | **已修复**：arguments 已覆盖（测试 `tool_call_arguments_are_scrubbed`）；`-----BEGIN`/`PRIVATE KEY-----` 整块 mask 到 END |
| C5（前半） | `env:VAR` 抽宿主机密注入插件无脱敏无 consent | **未核对**，保留在「仍开放」 |
| High | `decision_slot` 无 call-id，残留批准被下一个工具吞掉 | **已修复**：槽位为 `Mutex<Option<(u64, bool)>>`（call-id + 决定） |
| High | `sanitize_for_sample` 对原生 function-calling provider 扁平化 `tool_calls` → 非法历史 | **已修复**：仅 `!native_tool_calls` 时扁平化 |
| High | compaction splice 不感知 tool_call/result 配对 → orphan `Role::Tool` | **已修复**：切点感知配对；测试 `plan_keeps_an_over_budget_trailing_tool_block`（"kept tail starts at the owning call"） |
| High | UTF-8 截断 panic（CJK 可触发） | **已修复**：统一 `tools::util::{truncate,head,tail}` 按字符边界裁剪，bash/fs_read/test_runner 已改用 |
| Medium | hunks 三份全文、无上限 → OOM | **已修复**：`MAX_HUNK_CONTENT`(2 MiB) + `MAX_TOTAL_HUNKS`(10k)，逐出最旧并标 `content_dropped` |
| Medium | bwrap 资源限额全缺 / 无 `--unshare-pid` | **已修复**：argv 含 `--unshare-pid`；限额翻译为 `ulimit -v`/`ulimit -u` |
| Medium | `session_store` 每轮整份 append、无界增长 | **已修复**：超过 `MAX_HISTORY_BYTES`(8 MiB) 用「同目录临时文件 + rename」原子重写为最后一行，读路径语义不变。测试 `snapshot_rotation_keeps_only_the_newest_line` |
| Medium | `smart_test_runner` 绕过 sandbox+audit，超时不杀子进程 | **已修复**：与 bash 共用新增的 `tools/sandbox_cfg.rs`（audit→SandboxPlan→SandboxConfig）与 `ctx.sandbox.run_command`（env 清洗、限额、超时 tree-kill），保留 PASS/FAIL 头 + failure distill，并输出 audit verdict 与 timeout 行。测试 `dispatches_through_sandbox_and_keeps_only_failures`、`timeout_is_reported_as_tree_killed` |
| Medium | memory 注入 `replace("(no memory)")` 永不生效 → 召回块冻结 | **已修复**：模板加 `<!-- memory -->` / `<!-- /memory -->` 定界标记，spawn 与每轮都经 `set_memory_block`；无标记的旧会话退回旧字面替换（不破坏重放）。测试 4 个 |
| Medium | MCP pending oneshot 泄漏 / string id 折算为 0 / `export_tools` 锁竞争返回空表 | **已修复**：`PendingSlot` drop guard 清槽；`response_id()` 归一化数字与数字字符串（stdio reader + HTTP SSE 两处）；工具表改为 `RwLock<Arc<Vec<Value>>>` 缓存，仅成功 `tools/list` 后更新。测试 `response_id_accepts_numbers_and_numeric_strings`、`pending_slot_is_removed_on_drop` |
| Medium | 死接缝 `AgentChannels`、`AmbientProbe`、`should_prefire`/`PREFIRE_LEAD_PERCENT`、`SteerMark`、workspace `notify` 依赖 | **已删除**：`steering.rs` 整个模块随之移除（其 watcher 从未存在——这正是 C2 标志恒为 false 的根因）；`AgentEvent` 保留为 kernel 内部反馈类型；`notify` 从 workspace 依赖中移除（以上各项的 grep 计数均为 0）。 |
| 通用性 | `is_file_edit` 靠名字硬编码表而非工具元数据 | **已修复**：`decide()` 增加 `class: Option<ToolClass>`，判定为 `class == WorkspaceMutation \|\| <旧表兜底>`，engine 从 registry spec 取 class。测试 `accept_edits_trusts_the_tool_class` |
| 通用性 | steering 溯源 `steered_with` 恒为 None（distiller 分支是死代码） | **已修复**：engine 记录本 turn 实际注入的 steer（`TurnOutcome.steers` + `steers_this_turn()`，错误返回也保留），session 以 `"; "` 拼接填入 `TurnRecord.steered_with`（单测 `steers_join_into_one_correction_string`）。 |
| 规范差距 | 30fps `StreamThrottler` 死代码 | **已移除**（全仓库无 `throttler` 引用） |
| 规范差距 | `en:` 配置拼写错误静默降级成明文 key | **已消失**：现为 `env:`（`strip_prefix("env:")`） |
| 测试 | plan 模式无端到端用例 | **已补**：`tests/plan_mode.rs`——plan schema 含 `submit_plan` 且不含写/进程工具；`submit_plan` → `PlanSubmitted` 事件 + 持久化 `NoticeKind::Plan` 行；未提交只 nudge 一次且回合正常结束。 |
| 测试 | 一个 skills 目录测试丢了 `#[tokio::test]`，静默不跑 | **已修**：`stage4` 的 `skill_catalog_is_in_the_prompt_and_refreshes` 恢复运行（stage-4 由 6 个用例变 7 个） |

### 仍开放（已核对，本轮未修）

| 领域 | 结论 |
|---|---|
| `PluginPermissions` 声明后零消费 | **仍开放**：`manifest.rs` 声明并填默认值，但 network/fs 白名单不传给 MCP spawn、`sandboxed` 读了不用 |
| `wait_for_decision` 300s 等待期间 steering/cancel 不可达 | **仍开放**（`engine.rs` `TIMEOUT = 300s`） |
| distill 在 `tokio::spawn` 内做同步 redb 写 | **仍开放**：`distill.rs` 仍是 `tokio::spawn`（宜 `spawn_blocking`） |
| `MAX_RETRIES = 3` 的语义（实为初次 + 2 次重试） | **仍开放**（命名与行为不一致） |
| `ToolCallAssembler` 无 slot/index 上限（`index=999999` → OOM） | **仍开放**（未找到上限） |
| `Result<_, String>` 错误类型（memory store 全 API） | **仍开放**：本轮改动保持原签名 |
| `TurnRecord.outcome: String` 而非 enum | **仍开放** |
| memory 语义检索规范（LibSQL+FastEmbed/ANN） | **仍开放**：仍是 redb + hash-embed，`recall_facts` O(N) 扫描；`embed_dim`/`k=8`/`last-3`/`GIT_SNAPSHOT_BUDGET` 等仍硬编码 |
| tree-sitter 系工具 / `pty_session` / LSP / WASM 插件运行时 | **仍开放** |
| 跨平台 sandbox（macOS/Windows）、landlock/seccomp、CoW 快照 | **仍开放**：仅 Linux bwrap + none |
| codec 版本协商 / `#[serde(other)]` 容忍 | **仍开放**：`codec.rs` 保留为进程外前端契约（树内无消费者） |
| `UiCommand::UndoLastTurn` 前端仍未发送 | **仍开放**（`Cancel`/`SetModel` 已发送） |
| write-actor（WAL + 单写者） | **仍开放**：仍靠 Mutex/单写者约定 |
| app-desktop（iced）侧问题（update 每 delta 全量重解析、字体链、截断提示、双重打印等） | **仍开放**，但当前主线 UI 是 `crates/app-tauri`（React/Vite），`app-desktop` 是旧 iced 壳——这些结论只对旧壳成立 |

---

## 评审正文（原始快照）

> 以下为原始评审在评审时点写下的内容（plan 功能与上述多数修复尚未落地），保留以便追溯；逐条结论请以「核对状态」为准。

## 总评

架构分层干净、crate 单向依赖纪律好、纯 Rust（零 `*-sys`）约束达成、测试覆盖 happy path。
存在 **5 个会丢用户数据的 Critical 缺陷**、若干**审批/取消机制的结构性漏洞**，以及一批"声明了但没接线"的半成品（sandbox plan、plugin permissions、throttler、Cancel、WASM）。

---

## 一、Critical — 会丢数据 / 必须优先修

| # | 位置 | 问题 |
|---|------|------|
| C1 | `memory/store.rs:119,276` | **`next_id` 重启后从 1 重排**：`MemoryStore::open` 固定 `Mutex::new(1)`，不扫已有 max(id)。重开 DB 后 `alloc_id` 重新从 1 分配，`write_fact`/`add_episode` 用同 id `insert` **静默覆盖历史 facts/episodes**。跨 session 持久化实际是破坏性的。 |
| C2 | `hunks.rs:155` + `session.rs:457` | **Undo 不检查外部修改**：`undo_plan` 无条件恢复 `first hunk.old`。agent 写完→用户/外部又改→Undo 直接覆盖外部修改。`external_changes` 计数器存在但 undo 路径不查，且无 watcher 调 `handle_external_change`（永远=0）。 |
| C3 | `engine.rs:424,457` + `hunks.rs:28` | **非 UTF-8/二进制文件写入不进 undo**：`record_write` 用 `read_to_string`，二进制 Err→跳过。`Hunk` 用 `String` 非 `Vec<u8>`，undo 二进制文件不可能正确。 |
| C4 | `session.rs:573` + `engine.rs` | **`Cancel` 是空操作**：注释声称走 `EngineIo::cancel` flag，但该 flag 不存在。`run_turn` 在 `sampler.sample()`/`dispatch()` 中无 `select!`/取消检查 → 600s bash 或挂起 stream 完全无法中断。 |
| C5 | `mcp.rs:88-93` + `sampler.rs:160` | **`env:VAR` 抽宿主机密注入插件**无脱敏无 consent；**egress masker 只 scrub `content` 不 scrub `tool_calls[].arguments`** —— 工具把 secret 读出来塞回 arguments 是最可能的泄漏路径。 |

---

## 二、High — 审批/协议/正确性

### 审批与取消机制的系统性脆弱（跨 kernel+desktop）
- `decision_slot` 是裸 `Mutex<Option<bool>>`，**无 call-id 关联**。`session.rs:588` 无条件覆写、`engine.rs:539` `.take()` 首个非 None。用户对已超时卡片连点 → 残留 `Some(true)` 被下一个 `wait_for_decision` 吞掉 → **误批准无关工具**（bridge.rs:23 + update.rs:28）。
- `UiEvent` 全部 `try_send` 进 cap=256 channel，token 洪峰时 `ApprovalRequested`/`Error` 可被丢 → 胶囊永久 `Running`，engine 干等 300s。
- `update.rs:323` `pending` 单槽，第二个 `ApprovalRequested` 覆盖首个未决批准 → 批错胶囊。

### 协议正确性
- `compaction.rs:68` + `engine.rs:166`：每轮 sample 前 `sanitize_for_sample` 把 assistant `tool_calls` 扁平化成 `[call:…]` 文本，但对应 `Role::Tool` 结果仍在历史 → 对原生 function-calling provider（OpenAI/Anthropic）是**非法历史**（orphan tool result），且是 prompt-cache buster。应只在 text-protocol fallback 时扁平化。
- `compaction.rs:166`：`splice(0..prefix_end)` 切点不感知 tool_call/result 配对边界 → orphan `Role::Tool` → provider `invalid_argument`。
- `masking.rs:68-86`：`sk-`/`xai-`/`AKIA` 子串匹配**无词边界**误伤正常文本；`-----BEGIN` 只 mask 首行，**PEM 私钥正文泄漏**。
- `fs_patch.rs:74`：fuzzy_patch 在 tool 内 `fs::write` 直接写盘，permission 只管"是否 dispatch"——批准后副作用不可撤；`ApprovalRequested.fuzzy` 硬编码 `false`（engine.rs:397），审批卡无法提示"近似匹配"。

### UTF-8 边界 panic（release 是 panic=abort，直接崩）
`store.rs:337` `truncate(2048)`、`bash.rs:112` 字节切片、`test_runner.rs:142`、`openai_responses.rs:255` CallStripper 字节游标 —— 凡 `String` 索引/`truncate` 落在多字节字符中间即 panic，CJK 可触发。需统一 `truncate_utf8` helper。

---

## 三、Medium — 资源/并发/沙箱

- `hunks.rs:88`：每次写存 `old`+`new`+`unified` 三份全文，无上限无逐出 → OOM。
- `store.rs:155`：`upsert_fact` 跨 read/write txn，背靠背可双双过 dedupe → 重复 fact。应单 txn。
- `linux_bwrap.rs`：**资源限额全缺**（plan.rs 的 max_memory/max_processes 从未翻译成 `--rlimit`/cgroup）；`--proc`+`--dev` 全开无 `--unshare-pid` → 可读 `/proc/*/environ`、kill 宿主进程；workspace bind 不防 symlink 逃逸；timeout 无 pgid/cgroup 兜底，孙进程逃逸成孤儿。
- `plan.rs` **孤儿**：`SandboxPlan::from_audit` 的 deny/rlimit/AllowHosts 完全没被 `LinuxBwrap::run_command` 消费。
- `mcp.rs:162-230`：pending oneshot 泄漏（timeout 后 id 不删、map 膨胀）、子进程死无检测无重启、string-type JSON-RPC id 响应被 `as_u64().unwrap_or(0)` 丢弃。
- `update.rs:368`：每 delta 对全量 text 重跑 `markdown::parse` → 流式 O(n²)。
- `session_store.rs:66`：每轮整份 `Vec<ChatMessage>` append 一行 JSONL，无界增长，读时全文加载取末行。
- `sampler.rs:191`：流内 Err/Error 直接 return，**salvage/interrupted 未实现**（文档承诺的 partial text 回收没有）。
- `sampler.rs:122`：非可重试错误**不发 `SamplerEvent::Failed`**，静默失败。

---

## 四、通用性 / 设计债

- **manifest 权限形同虚设**：`PluginPermissions` 声明穷举上限但**零代码消费**——network/fs 白名单不传给 MCP spawn，`sandboxed` 字段读了不用。"未声明物理拒绝"完全未落地。
- **codec 无版本/握手**：serde 外部标签枚举是 Rust 特有格式，跨语言端要手工复刻；旧端遇新 variant 硬错，无 `#[serde(other)]` 容忍；`Decoder` 遇坏帧不清缓冲 → 永久卡死。
- **重复代码**：`GenericOpenAiProvider`/`OpenAiResponsesProvider` HTTP 壳 ~60 行逐字重复；`resolve_provider` 在 bridge.rs 与 app-cli/main.rs 逐字复制（缺 frontend-common crate）。
- **死代码/半成品**：`throttler.rs` 整文件 `#[allow(dead_code)]`（30fps 合并未接线）；`channels.rs` `AgentChannels` 无人用；`UiCommand::Cancel/SetModel/UndoLastTurn` 前端从未发送；`/compact` 是 stub；`enable()` 不重建 router；`git.rs` 的 `unified` diff 字段存了但 undo 只走全文 restore。
- **硬编码无配置入口**：`context_window=256_000`、`embed_dim=256`、`k=8/last-3`、`GIT_SNAPSHOT_BUDGET=4KB`、`FUZZY_THRESHOLD=0.9`、audit 规则表。
- **平台覆盖缺口**：sandbox 只有 Linux bwrap + none，macOS sandbox-exec/Windows Job Object/landlock/seccomp/CoW 快照全缺；WASM 运行时未实现（MCP only）。
- **`Result<_, String>` 错误类型**（store.rs 全部 API）丢失错误分类，与 crate 内 `thiserror` 风格不一致。
- **`outcome` 用 `String` 非 enum**（distill.rs）：`"success"|"failed"|"steered"` 靠注释约束，极易拼错。

---

## 五、规范差距（声明了但没落地）

| 规范承诺 | 现状 |
|---|---|
| LibSQL+FastEmbed 分层 memory | redb + hash-embed（无语义相似度，dedupe≈精确去重，无 ANN，O(N) 扫描） |
| tree-sitter smart_read/outline、grep scope、find_references、pty_session、undo_hunk 工具 | 均未实现，fs_read outline 还是 regex 启发式 |
| WASM 插件沙箱（capability-gated WASI） | 未实现，只有 MCP stdio |
| "未声明权限物理拒绝" | permissions 声明后零消费 |
| CoW 快照/回滚、landlock、跨平台后端 | 全缺 |
| 30fps StreamThrottler | 死代码未接线 |
| Cancel/Steer 完整接线 | Cancel 空操作、steer 仅部分 |
| write-actor（WAL+单写者） | 注释自承未实现，靠 Mutex |

---

## 六、修复优先级

- **P0（数据安全）**：C1 next_id 扫 max、C2 undo 查 external_changes、C3 Hunk 支持二进制、C4 Cancel 真接线、C5 env 抽密 consent + masker 覆盖 arguments。
- **P1（机制正确）**：decision_slot 加 call-id、Cancel 接 EngineCtl channel + select!、sanitize 不扁平化原生 tool_calls（按 provider 能力分支）、fuzzy_patch 返回 patched_content 由 engine 批准后写盘。
- **P2（资源与健壮）**：hunks 加大小上限+逐出、bwrap 接 plan.rs 的 rlimit/unshare-pid、统一 UTF-8 安全截断、session_store 加大小上限。
- **P3（架构债）**：plugin permissions 接线、codec 加版本协商、抽 frontend-common、补 tree-sitter/WASM/跨平台 sandbox。

---

## 各模块详细发现

### agent-context（workspace/git/hunks/memory）
- 除上述 C1/C2/C3 外：`distill.rs:51` 在 `tokio::spawn` 内跑同步 redb 写 → 阻塞 executor（应 `spawn_blocking`）；`git.rs:255` `enforce_budget` 单条 >4KB 时超预算且 `truncated` 误报；`store.rs:210` `recent_episodes` 每轮 O(N) 全表扫描+sort；`store.rs:348` `workspace_id` hash 未 canonicalize 的路径（symlink/`..`/非 UTF-8 → 同工作区不同 partition）；`hunks.rs:113` `handle_external_change` 无调用方。
- 通用性：`sort_by_file_name` 跨目录顺序不稳定 → prompt 渲染抖动；`undo` 不按 `origin` 过滤（plugin 写被用户 `/undo` 回滚）。

### agent-llm（provider/sse/sampler/masking/adapters）
- 除上述外：`sampler.rs:125` `MAX_RETRIES=3` 实际只重试 2 次（attempt 3 直接 Exhausted）与文档不符；`openai_compat.rs:260` clean-close（vLLM/Ollama）合成 `Done{None,None}` 丢已收到的 usage；`AGENT_DUMP_REQ` 把含 messages 的 body dump 到 stderr 且 `=0`/`false` 也触发；`types.rs:88` `ToolCallAssembler` 无 index 上限，恶意 `index=999999` → OOM；`config.rs:124` `en:` 拼写错误静默降级成明文 key。
- 通用性：`id()` 返回 `&'static str` 无法区分同类型多实例；`tools: Option<serde_json::Value>` 把 JSON Schema 泄漏进 trait；reasoning 字段各 adapter 覆盖不一致（`reasoning_details`/Anthropic `thinking` 未覆盖）。

### agent-kernel（engine/session/permissions/compaction/tools）
- 除上述外：`run_prompt` 的 memory 注入 `c.replace("(no memory)", &block)` 永不生效（placeholder 已被 `initial_memory_block` 替换）且若生效则每轮在 system prompt 追加新 block 造成膨胀；`wait_for_decision` 300s 期间 steering/cancel 不可达；`estimate_tokens` 不区分 CJK（chars/4 低估 ~4×，80% 触发太晚）；`smart_test_runner` 绕过 sandbox+audit 裸 `sh -c`；`hunks.record_write` 只对 3 个写工具名生效，bash `>` 重定向/插件写不入 hunk；`channels.rs` `AgentChannels` 死代码。
- 通用性：`ToolError` 4 变体但插件 router 二次尝试时原始错误链丢失；`is_readonly`/`is_write` 靠名字硬编码表而非工具元数据。

### agent-sandbox（bwrap/audit/shell_ast/plan/capability）
- 除上述外：`run_dir` 可预测（`agent-run-<pid>`）+ symlink 攻击面；`env_sanitize` denylist 误杀正当变量（`_URL`/`_URI`/`PRIVATE` 子串）；`shell_ast` 解析绕过面大（`${VAR}`/反斜杠拼接/`&>`重定向漏检/无后缀脚本/`xargs sh`/`find -exec`）；`capability.rs` 漏判 `git branch -D`/`mkfs`/`wget|sh`/`iptables`/`mount`；`audit.rs` `risk_of` 只对 `"/"`/`"/etc"`/`"/usr"`/`"/bin"` 升级，`rm -rf ~` 只算 Medium。
- 通用性：`none.rs` 残留坏注释块；`detect_backend` 非 Linux 直接 none；新增后端成本高（bwrap argv 硬编码内联，无 plan→argv 翻译器）。

### agent-plugin（mcp/manager/manifest）
- 除上述外：`manager.rs:73` trust 路径未 canonicalize（symlink/`..` 可绕过 repo-local 校验），家目录插件免 consent；`manager.rs:96` tool 名冲突 namespacing 不对称（注册顺序决定谁劫持裸名）；`lib.rs:40` `export_tools try_lock().unwrap_or_default()` 锁竞争时返回空 tools（LLM 瞬间看到 0 工具）；`provide_context` 无大小限无权限检查，插件可注入任意 prompt-injection。
- 规范差距：WASM 运行时、slash commands、hooks（MCP 无法 intercept）、HTTP/SSE transport、OAuth、`mcpServers` 的 `type/url` 字段全缺。

### agent-ipc（codec/events）+ app-desktop/app-cli
- 除上述外：`lib.rs:5` 文档声称 `AgentEvent` 在此 crate 但 events.rs:8 说不在（文档漂移）；`UiCommand::Cancel/SetModel/UndoLastTurn` 前端从未发送（wire 契约与实现脱节）；`bridge.rs` `spawn()` 在 iced 主线程同步跑 `SessionStore::open`+`AppConfig::load`+provider 构建；每 session 起两个 tokio Runtime；`throttler.rs` `is_reasoning` 算出即弃，text/reasoning 混 buffer；`state.rs:400` `view_from_history` 把所有 Tool 结果标 `Success` 丢错误态；`view.rs:468` `diff_view take(80)` 静默截断无 "…N more"；`theme.rs` 硬编码 JetBrains Mono/Inter 无 CJK fallback 链；`app-cli` `TextDelta`+`AssistantMessage` 双重打印。
