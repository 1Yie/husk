+++
owns = ["crates/agent-capabilities/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host", "agent-tools-fs", "agent-tools-shell"]
+++
可选宿主能力适配器：把工作区、沙盒、桌面接到通用工具上下文上。

关键词：workspace ctx, sandbox ctx, desktop ctx, capability adapter, optional feature, app paths

## 职责
- `WorkspaceCtx`（root / resolve / cancel）与 `WorkspacePaths`（app paths / workspace state dir）（workspace.rs）
- `SandboxCtx` / `SandboxSlot` / `SandboxLimitsSlot`（sandbox feature，sandbox.rs）
- `DesktopCtx` / `DesktopSlot`（desktop feature，desktop.rs）；默认 features 为空

## 不做
- 不依赖内核：内核写 `impl Trait for ToolCtx`（本地类型 + 外来 trait）
- 不决定策略（开关、限额、挂载归宿主）

## 对外契约
- 能力以 trait + 扩展槽形式提供；工具包泛型到这些 trait，不命名后端类型
