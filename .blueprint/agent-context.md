+++
owns = ["crates/agent-context/**"]
forbidden_deps = ["agent-kernel", "agent-llm", "reqwest"]
+++
工作区感知层：文件树扫描、git 快照、写入追踪、分层记忆。

关键词：workspace scanner, file index, git snapshot, hunk tracker, undo, memory, redb, embeddings, turn distiller

## 职责
- `WorkspaceScanner` / `WorkspaceTree`（workspace.rs）、git 快照（git.rs）
- `HunkTracker`：按回合记录写入、外部改动标记与撤销计划（hunks.rs）
- 分层记忆存储（memory/，redb + 向量检索）与回合蒸馏接缝（`TurnDistiller`）

## 不做
- 不调用模型；蒸馏所需的模型调用由宿主（husk-host）注入
- 不含架构蓝图——blueprint 已归 agent-coding

## 对外契约
- `HunkTracker`：按回合记录写入，供撤销与宿主使用
- `MemoryStore` / `TurnDistiller`：记忆读写与回合蒸馏的接缝
