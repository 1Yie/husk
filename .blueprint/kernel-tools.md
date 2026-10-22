+++
owns = ["crates/agent-kernel/src/tools/**"]
invariants = ["工具输出必须经过截断预算", "内置工具只有通用会话工具，不含任何产品能力", "写类工具只暂存 Commit，不自行落盘"]
+++

内核内置工具与注册表：通用会话工具 + 扩展槽，具体能力由宿主注册。

关键词：tool registry, tool spec, extension slots, batch_execute, delegate subagent, ask_question, link_workspace, todo, host slots, tool dispatch, approval actions

## 职责
- `ToolRegistry` / `ToolSpec` / `ToolCtx` / `Extensions`（registry.rs）与扩展槽（host_slots.rs）
- 通用会话工具：`batch_execute`、`delegate`、`ask_question`、`link_workspace`、`todo`（batch / delegate / ask / link / todo）
- `link_workspace`（link.rs）：显式申请把工作区外的一个目录授予本会话。内核不知道「授权」是什么——它把 `(action, path, access)` 交给宿主装的 `ScopeGrantSlot`，宿主决定能不能给、给到什么程度。`Review.risk = RequiresApproval` 保证任何模式下都先问人；`class: HumanInteraction` 让子代理拿不到它
- `with_builtins` / `child_builtins`：内核只注册这五个通用工具

## 不做
- 不含文件、shell、web、桌面、office、serena 等具体能力（属于 agent-tools-*，由宿主注册）
- 不决定回合流程（属于 kernel-core）；不渲染 UI
- 不判断某个目录能不能被授予：没有宿主槽时 `link_workspace` 直接报「这个宿主不能关联目录」，不假装成功

## 对外契约
- `ToolSpec` 的 `readonly` / `ToolClass` / `network` / `staged` / `review` 决定权限、批量与提交行为；新增工具必须声明
- `ToolContext` / `Extensions` 是工具契约（agent-tool）在内核的实例化点
- `ScopeGrantSlot`：`Fn(action, path, access) -> Result<(), String>`。宿主装它就等于「支持关联工作区」；`has_scope_grant` 让调用方先问一句再决定要不要弹卡
