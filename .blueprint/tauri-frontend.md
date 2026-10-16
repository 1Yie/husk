+++
owns = ["crates/app-tauri/frontend/**"]
forbidden_deps = ["tauri-backend", "kernel-core", "kernel-tools"]
+++
Husk 桌面端 React 前端：聊天流、会话侧栏、设置与产物面板。

关键词：react, chat stream, plan card, settings, session sidebar, notification center, artifacts dock, apply-event, agent-ipc types, frontend

## 职责
- 渲染 `UiEvent`（`apply-event`）并发出命令；设置页与产物面板
- 前端类型与 `agent-ipc` 的事件契约保持一致
- 通知中心（`lib/notifications.ts`）：判定哪些事件值得通知，投递到铃铛与系统通知
- 通知点击导航：订阅后按序读取后端保留的点击目标，等待启动会话恢复完成再打开目标项目和会话，处理后确认消费。

## 不做
- 不含内核行为；只经 Tauri IPC 与后端交互
