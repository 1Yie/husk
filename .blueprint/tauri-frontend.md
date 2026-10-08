+++
owns = ["crates/app-tauri/frontend/**"]
forbidden_deps = ["tauri-backend", "kernel-core", "kernel-tools"]
+++
Husk 桌面端 React 前端：聊天流、会话侧栏、设置与产物面板。

关键词：react, chat stream, plan card, settings, session sidebar, artifacts dock, apply-event, agent-ipc types, frontend

## 职责
- 渲染 `UiEvent`（`apply-event`）并发出命令；设置页与产物面板
- 前端类型与 `agent-ipc` 的事件契约保持一致

## 不做
- 不含内核行为；只经 Tauri IPC 与后端交互
