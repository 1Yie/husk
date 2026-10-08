+++
owns = ["crates/agent-kernel/src/tools/**"]
invariants = ["工具输出必须经过截断预算", "网络只用于 web_fetch 等显式工具（tools/net.rs）"]
+++
内置工具与模式化注册表。

关键词：tool registry, tool spec, apply_patch, fuzzy_patch, bash, smart_test_runner, web_fetch, delegate subagent, batch_execute, todo, plan tool, goal tool, office tools, blueprint tool, serena bridge

## 职责
- `ToolSpec`（`readonly` + `ToolClass`）与按模式过滤的注册表（build / plan / goal / office）
- 文件读写、补丁、grep、bash、测试运行器、桌面操作、web_fetch、子代理 delegate、todo、plan/goal 控制信号
- 写类工具只暂存 `PendingWrite`，落盘由 engine 在权限决定之后提交

## 不做
- 不自行落盘或绕过权限门；不决定回合流程（属于 kernel-core）
- 不渲染 UI

## 对外契约
- `ToolSpec` 的 `readonly` / `ToolClass` 决定权限与是否可批量并行；新增工具必须声明
