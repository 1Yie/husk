+++
owns = ["crates/app-tauri/*", "crates/app-tauri/{backend,capabilities,icons,linux}/**"]
forbidden_deps = ["ratatui*", "agent-plugin", "agent-computer", "tauri-frontend"]
+++
Husk 桌面端 Tauri 2 后端：内核之上的适配层与打包配置。

关键词：tauri, ipc commands, desktop shell, kernel bridge, tray, packaging, capabilities

## 职责
- IPC 命令桥（`backend/ipc`）：把前端调用转成 `UiCommand`、把 `UiEvent` 推给前端
- 窗口、托盘、单实例与打包配置

## 不做
- 不复制内核逻辑；行为变更先落在 kernel
- 不渲染界面（属于 tauri-frontend）
