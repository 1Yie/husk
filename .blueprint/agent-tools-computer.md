+++
owns = ["crates/agent-tools-computer/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host"]
+++
屏幕工具：截图与桌面操作，桌面后端是可选能力。

关键词：computer use, screenshot, desktop control, image input

## 职责
- `computer`（computer.rs）与 `screenshot`（screenshot.rs）
- 经 `DesktopCtx` 取宿主安装的桌面后端

## 不做
- 不决定何时操作桌面、不做审批判断（桌面审批归 agent-coding 的 ComputerPolicy）

## 对外契约
- 截图以 `ToolResult.images` 作为模型输入，落盘路径取 `WorkspacePaths::workspace_state_dir`
