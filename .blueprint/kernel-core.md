+++
owns = ["crates/agent-kernel/Cargo.toml", "crates/agent-kernel/{assets,tests,examples}/**", "crates/agent-kernel/src/{engine,session,session_manager,commands,compaction,channels,hooks,lib,mode,permissions,sandbox_prefs,blueprint}.rs"]
forbidden_deps = ["iced*", "tauri*", "ratatui*"]
invariants = ["文件写工具经权限门并由 HunkTracker 记录；bash 的写入不入 HunkTracker，新文件由回合起止的文件快照比对补查"]
+++
内核主干：ReAct 循环、会话、权限门、压缩、模式与回合末蓝图检查。

关键词：permission gate, ReAct loop, compaction, session, session manager, plan mode, goal mode, office mode, slash commands, system prompt, hooks chain, blueprint turn check

## 职责
- `AgentState` 状态机与回合循环（engine / session）
- 权限门、上下文压缩、模式（build / plan / goal / office）与斜杠命令
- 回合末蓝图检查与一次返工；goal 模式下撤回已宣告的完成
- 会话与多会话管理（SessionActor / SessionManager），插件 hook 链的编排

## 不做
- 不渲染任何 UI；前端只通过 agent-ipc 的 UiEvent/UiCommand 交互
- 模型请求一律经 agent-llm，不含厂商专有字段
- 不直接实现具体工具（属于 kernel-tools）

## 对外契约
- `SessionActor` / `SessionManager`：前端以 `UiCommand` 驱动，事件经 `UiEvent` 回流
