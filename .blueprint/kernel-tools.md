+++
owns = ["crates/agent-kernel/src/tools/**"]
invariants = ["工具输出必须经过截断预算", "内置工具只有通用会话工具，不含任何产品能力", "写类工具只暂存 Commit，不自行落盘"]
+++
内核内置工具与注册表：通用会话工具 + 扩展槽，具体能力由宿主注册。

关键词：tool registry, tool spec, extension slots, batch_execute, delegate subagent, ask_question, todo, host slots, tool dispatch

## 职责
- `ToolRegistry` / `ToolSpec` / `ToolCtx` / `Extensions`（registry.rs）与扩展槽（host_slots.rs）
- 通用会话工具：`batch_execute`、`delegate`、`ask_question`、`todo`（batch / delegate / ask / todo）
- `with_builtins` / `child_builtins`：内核只注册这四个通用工具

## 不做
- 不含文件、shell、web、桌面、office、serena 等具体能力（属于 agent-tools-*，由宿主注册）
- 不决定回合流程（属于 kernel-core）；不渲染 UI

## 对外契约
- `ToolSpec` 的 `readonly` / `ToolClass` / `network` / `staged` / `review` 决定权限、批量与提交行为；新增工具必须声明
- `ToolContext` / `Extensions` 是工具契约（agent-tool）在内核的实例化点
