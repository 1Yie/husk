+++
owns = ["crates/agent-tools-serena/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host"]
+++
serena 桥：把 serena 的 MCP 子进程包成一组代码智能工具。

关键词：serena, mcp bridge, code intelligence, stdio child, symbol tools

## 职责
- `spec()`（对 `WorkspaceCtx` 泛型）与 catalog / bridge / manager（tool / catalog / bridge / manager）
- 直接 spawn stdio 子进程，不经 agent-plugin

## 不做
- 不依赖内核与插件系统；只读宿主装进 ctx 的工作区

## 对外契约
- 与其它工具包同一模式：`spec()` 返回 `ToolSpec<C>`，由宿主注册
