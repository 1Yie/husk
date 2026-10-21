+++
owns = ["crates/agent-kernel/Cargo.toml", "crates/agent-kernel/src/{channels,compaction,engine,hooks,lib,permissions,plugins,turn_policy}.rs", "crates/agent-kernel/{tests,examples}/**"]
forbidden_deps = ["iced*", "tauri*", "ratatui*", "agent-coding", "husk-host"]
invariants = ["模型请求一律经 agent-llm 归一化的 StreamChunk，内核不含厂商字段", "写工具只暂存 Commit，落盘由引擎在权限决定之后执行", "内核不命名任何产品概念（plan / goal / blueprint / office / husk）"]
+++

内核主干：ReAct 回合循环、会话状态机、权限门、压缩、hook 链与插件路由——纯引擎，不含产品语义。

关键词：react loop, engine, agent state machine, permission gate, compaction, hooks chain, plugin router, turn policy, ui channels, mode table, cancel

## 职责
- `Engine` 的回合循环、工具派发与提交（engine.rs），`AgentState` 状态机
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

## 对外契约
- `Engine` / `ToolRegistry` / `TurnPolicy`：宿主装配后驱动；`Engine::new` 只装通用会话工具
- 内核正常依赖闭包 = agent-ipc / agent-llm / agent-tool
