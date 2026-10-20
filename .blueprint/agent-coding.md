+++
owns = ["crates/agent-coding/**"]
forbidden_deps = ["husk-host", "app-cli", "app-tui"]
invariants = ["内核不得反向依赖本 crate（agent-coding → agent-kernel 单向）", "goal / plan 控制器每上下文一套，绝不跨子代理共享"]
+++
编码产品语义：plan / goal / blueprint 回合契约、blueprint 读写工具与写守卫、编码回合策略。

关键词：plan mode, goal mode, blueprint, blueprint_edit, write guard, architecture check, coding policy, computer policy, remember, skill tool, write tracker

## 职责
- 回合契约与控制器：`goal` / `plan`（goal.rs / plan.rs），由 `install_contract` 安装
- `CodingPolicy` / `coding_policy_factory`（policy.rs）：催促上限、blueprint 返工、computer 动作限额、`ArchitectureSwitch`
- blueprint 解析/依赖图/检查（blueprint/mod.rs）：加载与渲染共用 `parse_node_file` / `render_node_file`，写出的节点必然是能读回的节点
- 节点唯一写入口 `blueprint_edit`（blueprint/edit.rs）：字段化修改、写入前校验、staged 提交，`reason` 随结果回显
- 写守卫 `guard_patch`（blueprint/guard.rs）：目标落在 `.blueprint/` 的调用即拒绝并指向 `blueprint_edit`，无论路径写在顶层（`path` / patch 头）还是元工具的 `arguments` 里
- 回合末基线（blueprint/runtime.rs）：`.blueprint/` 是隐藏目录、工作区遍历看不见，因此节点文件单独纳入快照；绕过工具改动的节点被点名
- 只读工具 `blueprint`（blueprint_tool.rs）：locate / read / check / graph
- 桌面审批 `ComputerPolicy`（computer_policy.rs）、写入追踪 `WriteTracker`（tracker.rs）
- 编码侧工具：`remember`（remember.rs）、`skill`（skill.rs）

## 不做
- 不装配会话（属于 husk-host）；不实现文件/shell 等能力（属于 agent-tools-*）
- 不依赖 husk-host 或任何前端

## 对外契约
- `MODE_PLAN` / `MODE_GOAL` 与宿主模式表共用；策略自持状态，经 `TurnPolicy` 接缝交给引擎
- `blueprint_tool::spec()` / `blueprint::edit::spec()` / `blueprint::guard_patch`：宿主注册读工具与写工具，并用守卫包住自己的 patch 工具
