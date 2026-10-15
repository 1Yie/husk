+++
owns = ["crates/agent-coding/**"]
forbidden_deps = ["husk-host", "app-cli", "app-tui"]
invariants = ["内核不得反向依赖本 crate（agent-coding → agent-kernel 单向）", "goal / plan 控制器每上下文一套，绝不跨子代理共享"]
+++
编码产品语义：plan / goal / blueprint 回合契约、blueprint 工具、编码回合策略。

关键词：plan mode, goal mode, blueprint, architecture check, coding policy, computer policy, remember, skill tool, write tracker

## 职责
- 回合契约与控制器：`goal` / `plan`（goal.rs / plan.rs），由 `install_contract` 安装
- `CodingPolicy` / `coding_policy_factory`（policy.rs）：催促上限、blueprint 返工、computer 动作限额、`ArchitectureSwitch`
- blueprint 解析/依赖图/检查（blueprint/）与 blueprint 工具（blueprint_tool.rs）
- 桌面审批 `ComputerPolicy`（computer_policy.rs）、写入追踪 `WriteTracker`（tracker.rs）
- 编码侧工具：`remember`（remember.rs）、`skill`（skill.rs）

## 不做
- 不装配会话（属于 husk-host）；不实现文件/shell 等能力（属于 agent-tools-*）
- 不依赖 husk-host 或任何前端

## 对外契约
- `MODE_PLAN` / `MODE_GOAL` 与宿主模式表共用；策略自持状态，经 `TurnPolicy` 接缝交给引擎
