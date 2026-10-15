+++
owns = ["crates/agent-store/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host", "agent-skills"]
invariants = ["旧 redb 数据只经校验过的快照导入，不加锁、不改动原文件"]
+++
会话持久化：基于文件的 JSONL 日志与原子索引。

关键词：session persistence, jsonl journal, session index, migration, recent workspaces, workspace prefs, global rows

## 职责
- `SessionStore` / `SessionDatabase`：每会话一份 JSONL 日志，锁只覆盖单次文件事务
- 会话索引与 meta、每工作区偏好行、最近工作区、全局行、旧数据迁移
- 数据目录由宿主经 `init_data_dir(_with_namespaces)` 注入

## 不做
- 不知道引擎、工具与权限；只存取已序列化的会话记录与线类型
