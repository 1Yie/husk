+++
owns = ["crates/husk-host/**"]
forbidden_deps = ["app-cli", "app-tui", "tauri*", "ratatui*"]
invariants = ["会话装配是宿主职责，内核与 agent-coding 不感知产品", "沙盒限额是进程级共享真相源，经 SwitchableSandbox 注入", "交互式会话不设工具轮次上限；无人值守的续跑由 MAX_WAKES_IN_ROW 把关，失控循环由 RepeatGuard 叫停", "记忆注入以 (scope,key) 记账；召回与语义推理不得占用异步 worker", "关联工作区按会话生效，绝不进进程级 sandbox_prefs::LIMITS；只经会话 ctx 的 LinkedRoots 槽注入", "关联根先经 link_policy::validate：不接受 /、$HOME 本身、系统目录、包含应用数据或凭据目录的祖先、不存在的路径", "授权只有一条写路径（links::LinkHub）：写入前校验、读回再校验、带锁读改写、改动刷新该工作区所有活会话", "link_policy::normalize 保留嵌套根：只读父 + 读写子不得合并成父级可写"]
+++

Husk 宿主层：把内核装配成产品——会话、命令面、模式表、沙盒偏好。

关键词：session assembly, session manager, session actor, commands, agent mode, sandbox prefs, subagents, subagent defaults, todo store, office root, system prompt, background jobs, memory distill, recall delta, recall prune, job log tail, semantic recall, turn budget, repeat guard, observation ledger, linked workspaces, link policy, link hub, scope grant

## 职责
- `SessionActor` / `SessionConfig` / `SessionManager`（session.rs / session_manager.rs）：装配 provider、registry、ctx、plugins、skills、sandbox、memory
- 命令面（command.rs 的 `UiCommand`、commands.rs 的斜杠命令）与模式表（mode.rs 的 `AgentMode`）
- 沙盒偏好（sandbox_prefs.rs）、插件适配（plugins.rs）、子代理宿主（subagents.rs）
- 关联工作区的策略（link_policy.rs）：`validate` 是唯一的准入判断（拒绝 `/`、`$HOME`、系统目录、包含工作区的祖先、应用数据与凭据目录及其祖先），`normalize` 让列表 canonical、按路径去重（同一路径取更宽的那份，嵌套路径**保留**——`scope_of` 取最具体根，所以只读父 + 读写子必须两条都在，否则等于把父级整棵树放开），`grant_target` 把「卡片上的文件」折成「要授予的目录」（项目根 → 目录本身 → 父目录），`same_path` 让已消失的目录仍能被取消关联
- 关联工作区的唯一写路径（links.rs）：`LinkHub` 一个工作区一份，经 `hub_for` 按 canonical 根共享——IPC 菜单、`link_workspace` 工具、越界卡片三处都调它，所以规则不会分叉。grant / revoke / set_access 各自是**带锁的读改写**（`SessionStore::update_prefs` / `update_session_links`），并发的授权不会互相覆盖；读回时逐条再跑 `validate`（手改的 prefs、旧版本写入、被换成指向 `~/.ssh` 的目录都不算数）；一次改动 `refresh` **该工作区所有活会话**的句柄并推 notice——否则「取消关联」只对当前窗口生效，别的窗口还能写
- 授权槽 `ScopeGrantSlot`（session.rs 装配）：内核问「这个路径能给吗」时，actor 把它转给同一个 `LinkHub`，所以引擎的重试立刻能看到新范围。它和 `link_workspace` 工具、以及「拒绝即提问」共用同一段逻辑——一条授权路径，不是三条
- 判决透传：`UiCommand::ToolDecision { request_id, approved, action }` 由 IPC 直接写进引擎的判决槽（不经命令泵），`action` 原样带到 `ToolVerdict`——卡上的「关联（本会话）」靠它变成一次真实授权
- 子代理模型路由（child_route.rs）：把所有已配置 provider 的模型并入内核 `ChildRoute` 的模型表（父 provider 用裸 id，全部 provider 用 `provider/model`），经 `RoutedProvider` 按键分发；全局子代理默认模型写入路由的 `model`
- 子代理显式设置的三层优先级：manifest `model:` / `thinking:` > 全局子代理默认模型与思考等级 > 父会话的模型与等级。模型与思考等级互相独立——一个子代理可以沿用父会话的模型却想得更少。等级在 `install` 里与默认模型一样写进 `ChildRoute`，且必须写在「无默认模型就 return」之前
- 子代理的预算来自内核常量（超时 15 分钟、不设轮次上限），宿主没有出口：它是一个独立的 agent，不是受会话预算节制的工具。系统提示里的预算文案随这些常量走，改一处必须改另一处
- 系统提示词资产（assets/kernel-system-prompt.md）、办公根目录（office.rs）、待办存储（todo_store.rs）、全局偏好（prefs.rs）。提示里有一段常驻的「如何在需要别的目录时申请」——它必须在没有任何关联时也存在，否则模型不知道这条路
- 提示词缓存布局：系统提示固定指令在前、会话上下文在后且会话中不改写（模式块除外），每条消息相关的记忆作为隐藏 user 行随消息发送；会话在回合结束与历史改写时通知保活（`WarmingControl`）
- 记忆注入记账：会话按历史索引记录已下发的事实 id 与已携带的 persona（`injected_facts` / `injected_persona`），召回行只发增量——同一事实一个会话只出现一次，会话中途 `remember` 的偏好在下一轮以同一行补发；persona 记账以 `<scope>:<key>` 为键，同名不同作用域是两条；`/retry`、`/clear`、`/compact` 回滚历史时同步丢弃对应记账
- 观察注记与历史同寿命：根上下文装一份 `ObservationLedger`，`forget_after`（`/clear`、`/retry`、`/compact`、hook 改写历史）整份清空——「你看过了」只对仍在上下文里的结果成立。子代理各拿自己的一份
- 召回行修剪（`prune_recall_rows_in`）：`/compact` 前只留最近两条召回行，被删行的记账与其索引一起前移。压缩本来就要重写历史，所以这是唯一不额外损失 prompt cache 的时机；自动压缩发生在内核内部，宿主不介入
- 召回在阻塞池上跑：`run_prompt` 把召回整段交给 `spawn_blocking`——语义模型就绪后第一次召回要补算整库向量，那是秒级 CPU，不能占运行时 worker
- 会话身份：`conversation_key`（本机随机盐 + 持久化会话 id + 工作区；盐存在全局行里，存储不可用时退化为进程内随机）传给本会话的每个 provider 构建点——会话 provider、降级链、后台 provider、子代理路由，重建时路由键不变
- 计数广播（`history_counts` / `emit_counts`）：assistant 行 = 步（一次模型请求及其工具），tool 行 = 调用；轮次取 store 的 `counts_as_turn`，随每次提交与 runs/tasks 一起下发，前端据此重定基准
- 后台任务呈现：会话持有自己的 `BgRegistry`，订阅其变更并下发全量快照 `Custom{kind:"husk:bg_jobs", payload:{jobs}}`；完成通知只进内核 notice 通道（模型侧），界面状态只走快照。每次变化时把记录（`bg_record`：快照 + 每个活任务的日志尾部 `log_tail`，`BG_RECORD_TAIL_LINES` 行、`BG_RECORD_TAIL_BYTES` 字节封顶）写入 store；actor 退出前再写一次，因为注册表随后会删掉日志文件。任务进程与日志文件随会话消亡，记录与日志尾部不随。推送给界面的快照不带日志尾部
- 后台任务记录跨进程：actor 启动时读入上次的记录（`restored_bg_jobs`：仍在跑的按「应用已退出」、全部标 `restored`、id 加 `@<started_at_ms>` 后缀以免与本进程从 `bg-1` 重新编号的任务撞车，只保留最近 `BG_RECORD_CAP` 条），之后快照、展示与落盘都是「记录 + 活注册表」，新进程的任务追加在旧记录之后而不是覆盖它
- 后台任务唤醒：notifier 在推送通知后 poke 一个唤醒通道；`run()` 在空闲时把通知变成一次自发的回合（`Engine::run_notice_turn`），用户提示优先于唤醒，唤醒回合不产生 user 行、不进记忆蒸馏。连续唤醒以 `MAX_WAKES_IN_ROW` 封顶、用户发言后清零，超出后通知留在通道里随下一次提示送达，无人值守时任务链不会自行续跑
- 回合预算：装配内核后调 `set_max_tool_rounds(usize::MAX)`，交互式会话不设轮次上限。一次大规模重构本来就可能跨几百次工具往返，而内核到点会收走工具逼模型「汇总已有信息」——对改代码的任务等于改到一半停手。真正设界的是上下文窗口（内核每轮的 sanitize/compaction）和用户的取消；无人值守路径另有唤醒预算管着。轮次计数不是「agent 走偏」的信号，重复同一次调用才是——那个检查在 `repeat_guard.rs`
- 循环兜底（repeat_guard.rs）：每会话一个 `RepeatGuard`，作为 hook 的 **local 层**装在 `HookChain::layered` 里（不是插件，`reload_plugins` 不会把它换掉）。同一个工具 + 同样的参数 + 同样的输出连续出现到第 4 次结果后，下一次同调用在 `before_tool_execute` 被拒绝，并给出「换办法或报告阻塞」的指引。streak 只认**连续**：中间任何别的调用（尤其是一次编辑）都会清零，所以「改完再跑同一个测试」不会被误判
- 记忆蒸馏：`build_distiller` 是唯一构造点（spawn / `SetMemory` / 模型切换共用），`DistillCtx` 随会话模型更新，所以归纳用的是当前模型；回合记录带上最终回答（`last_reply`）作为归纳证据；每次蒸馏的 token 经 `distill_record` 写进 store 的侧通道，统计页可见但不计入会话用量
- 语义召回（`semantic` feature）：`SessionManager::spawn_at` 与 `set_memory_prefs` / `set_semantic_recall` 统一经 `apply_semantic_switch` 调 `agent_context::memory::semantic::set_enabled(memory_enabled && semantic_recall, data_dir/models)`——编码器只在两道开关都开时加载，关记忆即关编码器；本 crate 不直接依赖 fastembed，`semantic` 只是转发到 `agent-context/semantic`

## 不做
- 不复制内核逻辑；行为变更先落在 kernel 或 agent-coding
- 不被任何前端反向依赖（前端依赖它）
- 不自己实现嵌入：模型与服务都在 agent-context
- 不把整表召回或语义推理放在运行时 worker 上
- 不在宿主侧重写工具描述：工具职责与能力属于 agent-tools-*；这里只加与本次对话相关的注记
- 不把关联工作区放进进程级 `sandbox_prefs`：那样一个会话的许可会漏给同进程的所有会话
- 不复制一份授权逻辑：`link_workspace` 工具、越界卡片的按钮、composer 菜单，走的都是 `links::LinkHub` + `link_policy`
- 不让授权只对一个会话生效：写授权就等于写「这个工作区的活会话都该知道」，`refresh` 是唯一出口

## 对外契约
- `SessionManager` / `SessionActor` 以 `UiCommand` 驱动、`UiEvent` 回流；`base_registries()` 组装完整工具集（patch 工具与 `serena` 元工具在此经 `agent_coding::blueprint::guard_patch` 包装，`.blueprint/` 只能由 `blueprint_edit` 写入；观察工具经 `agent_coding::observe::{read, grep, list, bash}` 包装以追加注记；记忆读写工具 `remember` / `memory_search` 同在这里注册）。观察包装**不得**把 `ToolError::OutOfScope` 拍平成 `Failed`——那会把授权卡吞掉
- 关联工作区：`links::hub_for(root, paths)` 交出该工作区唯一的 `LinkHub`；`LinkHub::{effective, layers, grant, revoke, set_access, register, refresh}`。`Layer::{Session, Workspace}` 由卡片 action id 决定（`link:session` / `link:workspace`，未知 id 一律拒绝）。`persist: true` 写工作区偏好（该工作区的每个会话都继承），`false` 只写本会话文件。取消关联会把两层里同一路径都删掉——「取消」的意思是这个会话不再能到达它
- 斜杠命令 `/remember`（全局作用域）、`/forget`（两个作用域）、`/memory`（列出 persona 并标注作用域，`/memory purge` 走 `MemoryStore::purge_polluted` 迁移旧库）
- 子代理默认：`SessionManager::set_subagent_model` / `set_subagent_thinking_level` 各写一份全局偏好，并一起经 `broadcast_subagent_defaults` 推给每个活/挂起 actor（`UiCommand::SetSubagentModel` / `UiCommand::SetSubagentThinking`）；空值表示「跟随父会话」
- 子代理清单：`subagents::render_manifest` 写 `model:` 与 `thinking:` 两行；编辑器拥有的 key 先从 `existing` 里剔除，其余原样保留，所以保存不会丢手写的 `tools:` / `readonly:`。`read_subagent` 回读两者
- 后台任务：`SessionManager::bg_registry(root, id)` 交出会话的 `BgRegistry`，前端适配层据此读日志与控制（kill / pause / resume / remove），不经 actor 命令泵，回合进行中也即时生效；界面上的 kill 会以「被用户停止」的通知告知模型
- 后台任务列表：`bg_jobs_for_display` 在 actor 存活时给「恢复的记录 + 活注册表」，否则直接读记录文件并按同一规则恢复。记录态只能整条删除（`remove_bg_record`：actor 存活时只认恢复条目、命中才返回 `Some`，否则交给活注册表），不能暂停/终止；日志只剩记录里保存的 `log_tail`
- `SessionManager::active_store()`：交出当前工作区的 `SessionStore` 句柄（无工作区时 `None`），供适配层在管理器锁外做整份日志的读取（如 `SessionStore::tool_results`）
- 侧路流量（蒸馏、缓存预热）一律走不带 usage observer 的 provider 实例（`unobserved_provider_from`），只有前台回合写会话的 cache-write 计数
