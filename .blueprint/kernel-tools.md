+++
owns = ["crates/agent-kernel/src/tools/**"]
invariants = ["工具输出必须经过截断预算", "内置工具只有通用会话工具，不含任何产品能力", "写类工具只暂存 Commit，不自行落盘"]
+++

内核内置工具与注册表：通用会话工具 + 扩展槽，具体能力由宿主注册。

关键词：tool registry, tool spec, extension slots, batch_execute, delegate subagent, ask_question, link_workspace, todo, host slots, tool dispatch, approval actions, child approval

## 职责
- `ToolRegistry` / `ToolSpec` / `ToolCtx` / `Extensions`（registry.rs）与扩展槽（host_slots.rs）
- 通用会话工具：`batch_execute`、`delegate`、`ask_question`、`link_workspace`、`todo`（batch / delegate / ask / link / todo）
- `link_workspace`（link.rs）：显式申请把工作区外的一个目录授予本会话。内核不知道「授权」是什么——它把 `(action, path, access)` 交给宿主装的 `ScopeGrantSlot`，宿主决定能不能给、给到什么程度。`Review.risk = RequiresApproval` 保证任何模式下都先问人；`class: HumanInteraction` 让子代理拿不到它
- `delegate`（delegate.rs）：子代理是完整的一次内核回合，跑在自己的 task 里。`with_parent_gate` 把父会话的权限门交给它，子代理按**同一套**模式与规则判定（装配点见 kernel-core）；`with_child_approvals` 由宿主声明「审批能被用户看到」，只有这时子代理才会停下来等卡。子代理画的审批卡落在父流上，带自己的 `child_id`
- `route_child_approval` / `deliver_child_decision`：审批请求号是进程级唯一的，这张表把用户点的那一下送回**真正在等它的那个子引擎**
- `with_builtins` / `child_builtins`：内核只注册这五个通用工具

## 不做
- 不含文件、shell、web、桌面、office、serena 等具体能力（属于 agent-tools-*，由宿主注册）
- 不决定回合流程（属于 kernel-core）；不渲染 UI
- 不判断某个目录能不能被授予：没有宿主槽时 `link_workspace` 直接报「这个宿主不能关联目录」，不假装成功
- 不替用户回答子代理的审批：内核只负责把问题送达、把答案送回，答不答是人的事

## 对外契约
- `ToolSpec` 的 `readonly` / `ToolClass` / `network` / `staged` / `review` 决定权限、批量与提交行为；新增工具必须声明
- `ToolContext` / `Extensions` 是工具契约（agent-tool）在内核的实例化点
- `ScopeGrantSlot`：`Fn(action, path, access) -> Result<(), String>`。宿主装它就等于「支持关联工作区」；`has_scope_grant` 让调用方先问一句再决定要不要弹卡
- `install_child_notifier`：平行派发提前返回后，还在跑的子代理用它把报告交回父会话（宿主接到通知通道上）
