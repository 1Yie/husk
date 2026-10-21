+++
owns = ["crates/husk-host/**"]
forbidden_deps = ["app-cli", "app-tui", "tauri*", "ratatui*"]
invariants = ["会话装配是宿主职责，内核与 agent-coding 不感知产品", "沙盒限额是进程级共享真相源，经 SwitchableSandbox 注入"]
+++

Husk 宿主层：把内核装配成产品——会话、命令面、模式表、沙盒偏好。

关键词：session assembly, session manager, session actor, commands, agent mode, sandbox prefs, subagents, todo store, office root, system prompt, background jobs, memory distill

## 职责
- `SessionActor` / `SessionConfig` / `SessionManager`（session.rs / session_manager.rs）：装配 provider、registry、ctx、plugins、skills、sandbox、memory
- 命令面（command.rs 的 `UiCommand`、commands.rs 的斜杠命令）与模式表（mode.rs 的 `AgentMode`）
- 沙盒偏好（sandbox_prefs.rs）、插件适配（plugins.rs）、子代理宿主（subagents.rs）
- 子代理模型路由（child_route.rs）：把所有已配置 provider 的模型并入内核 `ChildRoute` 的模型表（父 provider 用裸 id，全部 provider 用 `provider/model`），经 `RoutedProvider` 按键分发；全局子代理默认模型写入路由的 `model`，优先级为 manifest `model:` > 全局子代理默认 > 父会话模型
- 系统提示词资产（assets/kernel-system-prompt.md）、办公根目录（office.rs）、待办存储（todo_store.rs）、全局偏好（prefs.rs）
- 提示词缓存布局：系统提示固定指令在前、会话上下文在后且会话中不改写（模式块除外），每条消息相关的记忆作为隐藏 user 行随消息发送；会话在回合结束与历史改写时通知保活（`WarmingControl`）
- 会话身份：`conversation_key`（本机随机盐 + 持久化会话 id + 工作区；盐存在全局行里，存储不可用时退化为进程内随机）传给本会话的每个 provider 构建点——会话 provider、降级链、后台 provider、子代理路由，重建时路由键不变
- 计数广播（`history_counts` / `emit_counts`）：assistant 行 = 步（一次模型请求及其工具），tool 行 = 调用；轮次取 store 的 `counts_as_turn`，随每次提交与 runs/tasks 一起下发，前端据此重定基准
- 后台任务呈现：会话持有自己的 `BgRegistry`，订阅其变更并下发全量快照 `Custom{kind:"husk:bg_jobs", payload:{jobs}}`；完成通知只进内核 notice 通道（模型侧），界面状态只走快照
- 后台任务唤醒：notifier 在推送通知后 poke 一个唤醒通道；`run()` 在空闲时把通知变成一次自发的回合（`Engine::run_notice_turn`），用户提示优先于唤醒，唤醒回合不产生 user 行、不进记忆蒸馏。连续唤醒以 `MAX_WAKES_IN_ROW` 封顶、用户发言后清零，超出后通知留在通道里随下一次提示送达，无人值守时任务链不会自行续跑
- 记忆蒸馏：`build_distiller` 是唯一构造点（spawn / `SetMemory` / 模型切换共用），`DistillCtx` 随会话模型更新，所以归纳用的是当前模型；每次蒸馏的 token 经 `distill_record` 写进 store 的侧通道，统计页可见但不计入会话用量

## 不做
- 不复制内核逻辑；行为变更先落在 kernel 或 agent-coding
- 不被任何前端反向依赖（前端依赖它）

## 对外契约
- `SessionManager` / `SessionActor` 以 `UiCommand` 驱动、`UiEvent` 回流；`base_registries()` 组装完整工具集（patch 工具与 `serena` 元工具在此经 `agent_coding::blueprint::guard_patch` 包装，`.blueprint/` 只能由 `blueprint_edit` 写入）
- 后台任务：`SessionManager::bg_registry(root, id)` 交出会话的 `BgRegistry`，前端适配层据此列出、读日志与控制（kill / pause / resume / remove），不经 actor 命令泵，回合进行中也即时生效；界面上的 kill 会以「被用户停止」的通知告知模型
- 侧路流量（蒸馏、缓存预热）一律走不带 usage observer 的 provider 实例（`unobserved_provider_from`），只有前台回合写会话的 cache-write 计数
