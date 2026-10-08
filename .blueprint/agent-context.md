+++
owns = ["crates/agent-context/**"]
forbidden_deps = ["agent-kernel", "agent-llm", "reqwest"]
+++
工作区感知层：文件树、git 快照、写入追踪、记忆、架构蓝图。

关键词：workspace scanner, file index, git snapshot, hunk tracker, undo, memory, redb, blueprint, architecture check

## 职责
- WorkspaceScanner / git 快照 / HunkTracker
- 分层记忆存储（redb）
- `.blueprint/` 解析、依赖图推导与规则检查

## 不做
- 不调用模型；蒸馏的模型调用由 kernel 注入

## 对外契约
- `HunkTracker`：按回合记录写入，供撤销与回合末蓝图检查使用
- `Blueprint::{load, locate, check, check_turn, graph}`：检查报告只含 error / warning，只有 error 触发返工
