+++
owns = ["crates/app-tauri/*", "crates/app-tauri/{backend,capabilities,icons,linux}/**"]
forbidden_deps = ["ratatui*", "agent-plugin", "agent-computer", "tauri-frontend"]
+++
Husk 桌面端 Tauri 2 后端：内核之上的适配层与打包配置。

关键词：tauri, ipc commands, desktop shell, kernel bridge, tray, system notification, packaging, capabilities

## 职责
- IPC 命令桥（`backend/ipc`）：把前端调用转成 `UiCommand`、把 `UiEvent` 推给前端
- 窗口、托盘、单实例、系统通知投递（`notify` op）与打包配置
- 系统通知的点击回传：唤起主窗口，把发送时附带的 `target` 经 `husk://notification-click` 交还前端；Linux/Windows 经 `notify-rust` 投递；macOS 11+ 直接使用 `UNUserNotificationCenter`，处理异步权限与持久通知的 delegate 回调，不依赖已弃用的 `NSUserNotificationCenter` 或借用 Terminal 身份。
- 点击队列通过 `peek_notification_click` / `ack_notification_click` 与前端交接，界面就绪前或重挂载期间的点击保留到导航处理完成。

## 不做
- 不复制内核逻辑；行为变更先落在 kernel
- 不渲染界面（属于 tauri-frontend）
