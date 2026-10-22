+++
owns = ["crates/agent-capabilities/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host", "agent-tools-fs", "agent-tools-shell"]
+++

可选宿主能力适配器：把工作区、沙盒、桌面接到通用工具上下文上。

关键词：workspace ctx, sandbox ctx, desktop ctx, capability adapter, optional feature, app paths, linked roots, cross workspace, out of scope

## 职责
- `WorkspaceCtx`（root / resolve / resolve_read / cancel）与 `WorkspacePaths`（app paths / workspace state dir）（workspace.rs）
- 关联根 `LinkedRoots` / `LinkedRoot { path, access }`（workspace.rs）：宿主授予的其他目录。`resolve_read` 接受主根或任一关联根，`resolve`（写路径）接受主根或 `ReadWrite` 关联根；只读根上的写入报 `ToolError::Failed` 并说明原因。相对路径永远相对主根
- 越界的**形状**归这里：落在所有根之外的路径返回 `ToolError::OutOfScope { path, suggestion }`，而不是 `PathEscape`——前者是「宿主也许能授予」的提问，后者留给真正的陷阱（悬空符号链接、指向别处的链接），跟着它们写就会写到不知道哪里。`suggest_root` 负责猜出该授予的项目根（`.git` / `Cargo.toml` / `package.json` / `pyproject.toml`）
- 关联根槽是 `Arc<RwLock<…>>` 的共享句柄：宿主替换内容即热生效，子代理继承同一句柄
- `SandboxCtx` / `SandboxSlot` / `SandboxLimitsSlot`（sandbox feature，sandbox.rs）
- `DesktopCtx` / `DesktopSlot`（desktop feature，desktop.rs）；默认 features 为空

## 不做
- 不依赖内核：内核写 `impl Trait for ToolCtx`（本地类型 + 外来 trait）
- 不决定策略（开关、限额、挂载、关联哪些目录、能不能授予归宿主）

## 对外契约
- 能力以 trait + 扩展槽形式提供；工具包泛型到这些 trait，不命名后端类型
- `WorkspaceCtx::roots()` 给出主根 + 关联根快照，供审计与沙盒挂载使用
- 只有「根外」用 `OutOfScope`；符号链接逃逸、只读根写入仍是各自的错误——调用方（引擎）只把前者变成授权卡
