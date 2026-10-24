+++
owns = ["crates/agent-kernel/Cargo.toml", "crates/agent-kernel/src/{channels,compaction,engine,hooks,lib,permissions,plugins,turn_policy}.rs", "crates/agent-kernel/{tests,examples}/**"]
forbidden_deps = ["iced*", "tauri*", "ratatui*", "agent-coding", "husk-host"]
invariants = ["模型请求一律经 agent-llm 归一化的 StreamChunk，内核不含厂商字段", "写工具只暂存 Commit，落盘由引擎在权限决定之后执行", "内核不命名任何产品概念（plan / goal / blueprint / office / husk）", "越界授权的 access 跟着触发工具走：只读工具申请 read，写工具与 bash 申请 readwrite"]
+++

内核主干：ReAct 回合循环、会话状态机、权限门、压缩、hook 链与插件路由——纯引擎，不含产品语义。

关键词：react loop, engine, agent state machine, permission gate, compaction, hooks chain, plugin router, turn policy, ui channels, mode table, cancel, verdict, scope grant, child gate, approval routing

## 职责
- `Engine` 的回合循环、工具派发与提交（engine.rs），`AgentState` 状态机
- 判决槽 `ToolVerdict { approved, action }`：批准之外还带「用户点了卡上的哪个动作」。卡上有 actions 时，选中项由 `ScopeGrantSlot` 落地，然后是**同一次调用的重试**（见下）
- 越界即提问（`ask_scope_grant`）：工具返回 `ToolError::OutOfScope` 时，引擎把宿主能的授权做成审批卡的 actions（本会话 / 本工作区），用户选中就调用 `ToolCtx::grant_scope`，成功则**重跑同一次调用一次**。两处派发点都接：staged 工具的试运行、普通工具的实际执行。每调用最多重试一次——宿主给不出有用的授权时不会自我循环
- 授权范围跟着触发它的工具走：只读工具（`ToolRegistry::is_readonly`）越界时申请 `read`，写工具与 `bash` 申请 `readwrite`。一次「让我看看那个文件」不该变成「整个外部仓库可写」——用户点的是卡上写明的那个范围，卡片因此必须同时写出**要授予的目录**和**权限级别**，两者都不许由宿主之外的推断补齐
- 权限门的继承（permissions.rs `PermissionGate::child_of`）：子代理按父会话的**同一套**模式与规则判定。两条不复制的：用户的「本会话允许」点击（`session_grants`，回答的是父会话那一件任务）、`bypass`（子代理无人守着，降为 `auto`）。`can_ask` 只决定越界时是停在卡上还是当场拒绝，从不放宽能做什么
- 审批卡的归属：`ApprovalRequested` / `ApprovalTimedOut` 带 `child_id`，请求号进程级唯一；子引擎在等待前把 `(request_id, decision_slot)` 登记到 kernel-tools 的转发表，于是同一个点击能落回真正在等它的那个引擎
- 取消：`EngineIo::cancel` 在采样间隙与工具派发期间都生效——派发与标志竞速，取消时丢弃 future，沙盒后端随之杀掉整个进程组（bwrap 靠 `--die-with-parent`，其余后端靠组守卫）；被中断的调用仍补一条工具结果，只有已发出 `ToolCallStarted` 的调用才补发 `ToolCallFinished`，历史可重放
- `UiChannels` 事件/命令通道（channels.rs，对命令类型泛型）、权限门（permissions.rs）
- 上下文压缩（compaction.rs）、hook 链（hooks.rs）、插件路由接缝（plugins.rs 的 `PluginRouter` / `PluginHandle`）
- 回合策略接缝 `TurnPolicy` / `TurnView` / `Verdict`（turn_policy.rs）与宿主注入的模式表
- 工具注册表与扩展槽的内核侧实现（见 kernel-tools）

## 不做
- 不渲染 UI；只经 agent-ipc 的 UiEvent/UiCommand 交互
- 不含 Coding 语义（plan / goal / blueprint / office / computer 策略）——属于 agent-coding
- 不实现具体工具（属于 kernel-tools 与 agent-tools-*），不依赖任何 agent-tools-*
- 不持有会话装配、持久化、技能与用户设置——分别属于 husk-host / agent-store / agent-skills
- 不知道「关联工作区」是什么意思：越界只问宿主，宿主没装槽时就是一句普通拒绝

## 对外契约
- `Engine` / `ToolRegistry` / `TurnPolicy`：宿主装配后驱动；`Engine::new` 只装通用会话工具
- `decision_slot()` 交出 `Option<(u64, ToolVerdict)>`；宿主把 `ToolDecision { request_id, approved, action }` 直接写进去，绕过命令泵
- `set_permissions_shared(Arc<RwLock<PermissionGate>>)`：宿主与子代理派发器共享同一个门，回合中途切模式连已经派出去的子代理一起生效
- `ToolCtx::{has_scope_grant, grant_scope(action, path, access)}`：`access` 由引擎按 `is_readonly` 决定，宿主只负责把它落地
- 内核正常依赖闭包 = agent-ipc / agent-llm / agent-tool
