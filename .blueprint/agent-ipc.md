+++
owns = ["crates/agent-ipc/**"]
forbidden_deps = ["agent-kernel", "agent-llm", "tokio"]
invariants = ["只含可序列化的数据类型"]
+++

内核与所有前端之间的通道契约：UiCommand / UiEvent。

关键词：ui event, ui command, wire encoding, serde, channel contract, approval actions

## 职责
- 事件与命令的类型定义和线编码
- `ApprovalRequested.actions`（`ApprovalAction { id, label, description }`）：卡片的**附加动作**，不是「允许/拒绝」的替代。空列表 = 普通审批卡。`id` 对前端不透明——它随 `ToolDecision` 回去，只有提出它的宿主知道含义。带 `serde(default)`，旧读取方看到的是空列表

## 不做
- 不含行为逻辑
- 不认识任何 action 的含义（`link:session` 这类字符串是宿主的词汇，不是契约的）

## 对外契约
- `UiCommand`（前端 → 内核）、`UiEvent`（内核 → 前端）；新增变体需同步三个前端
- 判决方向：`ToolDecision { request_id, approved, action }`——`action` 缺省即普通批准；三个前端都必须原样转发，否则「点了授权按钮」会退化成「批准」
