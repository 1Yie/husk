+++
owns = ["crates/agent-ipc/**"]
forbidden_deps = ["agent-kernel", "agent-llm", "tokio"]
invariants = ["只含可序列化的数据类型"]
+++
内核与所有前端之间的通道契约：UiCommand / UiEvent。

关键词：ui event, ui command, wire encoding, serde, channel contract

## 职责
- 事件与命令的类型定义和线编码

## 不做
- 不含行为逻辑

## 对外契约
- `UiCommand`（前端 → 内核）、`UiEvent`（内核 → 前端）；新增变体需同步三个前端
