+++
owns = ["crates/app-tui/**"]
forbidden_deps = ["tauri*", "agent-plugin", "agent-computer", "agent-context"]
+++
ratatui 终端前端，与桌面端同一事件契约。

关键词：terminal ui, ratatui, inline viewport, scrollback

## 职责
- 渲染 `UiEvent`、采集输入并发出 `UiCommand`

## 不做
- 不复制内核逻辑；行为变更先落在 kernel
