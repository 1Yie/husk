+++
owns = ["crates/agent-kernel/src/session_store.rs", "crates/agent-kernel/src/session_store/**"]
forbidden_deps = ["kernel-core", "kernel-tools", "kernel-skills"]
invariants = ["旧 redb 数据只经校验过的快照导入，不加锁、不改动原文件"]
+++
会话持久化：基于文件的 JSONL 日志与原子索引。

关键词：session persistence, jsonl journal, session index, migration, recent workspaces, session database

## 职责
- `SessionDatabase`：每个会话一份独立 JSONL 日志，锁只覆盖单次文件事务
- 会话索引、最近工作区记录、旧 redb 数据迁移

## 不做
- 不知道引擎、工具与权限；只存取已序列化的会话记录
