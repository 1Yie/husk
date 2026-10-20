+++
owns = ["crates/husk-host/**"]
forbidden_deps = ["app-cli", "app-tui", "tauri*", "ratatui*"]
invariants = ["会话装配是宿主职责，内核与 agent-coding 不感知产品", "沙盒限额是进程级共享真相源，经 SwitchableSandbox 注入"]
+++
Husk 宿主层：把内核装配成产品——会话、命令面、模式表、沙盒偏好。

关键词：session assembly, session manager, session actor, commands, agent mode, sandbox prefs, subagents, todo store, office root, system prompt

## 职责
- `SessionActor` / `SessionConfig` / `SessionManager`（session.rs / session_manager.rs）：装配 provider、registry、ctx、plugins、skills、sandbox、memory
- 命令面（command.rs 的 `UiCommand`、commands.rs 的斜杠命令）与模式表（mode.rs 的 `AgentMode`）
- 沙盒偏好（sandbox_prefs.rs）、插件适配（plugins.rs）、子代理宿主（subagents.rs）
- 子代理模型路由（child_route.rs）：把所有已配置 provider 的模型并入内核 `ChildRoute` 的模型表（父 provider 用裸 id，全部 provider 用 `provider/model`），经 `RoutedProvider` 按键分发；全局子代理默认模型写入路由的 `model`，优先级为 manifest `model:` > 全局子代理默认 > 父会话模型
- 系统提示词资产（assets/kernel-system-prompt.md）、办公根目录（office.rs）、待办存储（todo_store.rs）、全局偏好（prefs.rs）
- 提示词缓存布局：系统提示固定指令在前、会话上下文在后且会话中不改写（模式块除外），每条消息相关的记忆作为隐藏 user 行随消息发送；会话在回合结束与历史改写时通知保活（`WarmingControl`）

## 不做
- 不复制内核逻辑；行为变更先落在 kernel 或 agent-coding
- 不被任何前端反向依赖（前端依赖它）

## 对外契约
- `SessionManager` / `SessionActor` 以 `UiCommand` 驱动、`UiEvent` 回流；`base_registries()` 组装完整工具集（patch 工具与 `serena` 元工具在此经 `agent_coding::blueprint::guard_patch` 包装，`.blueprint/` 只能由 `blueprint_edit` 写入）
