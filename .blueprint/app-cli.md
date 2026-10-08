+++
owns = ["crates/app-cli/**"]
forbidden_deps = ["tauri*", "ratatui*", "agent-plugin", "agent-computer", "agent-context"]
+++
Headless CLI 前端。

关键词：headless, one-shot prompt, jsonl events, exit code, ci

## 职责
- 把 `UiEvent` 泵到 stdout；`--prompt` 一次性运行并以真实退出码结束

## 不做
- 不复制内核逻辑；行为变更先落在 kernel
