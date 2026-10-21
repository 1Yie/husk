+++
owns = ["crates/agent-store/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host", "agent-skills"]
invariants = ["旧 redb 数据只经校验过的快照导入，不加锁、不改动原文件"]
+++

会话持久化：基于文件的 JSONL 日志与原子索引。

关键词：session persistence, jsonl journal, session index, migration, recent workspaces, workspace prefs, global rows, distill usage

## 职责
- `SessionStore` / `SessionDatabase`：每会话一份 JSONL 日志，锁只覆盖单次文件事务
- 会话索引与 meta、每工作区偏好行、最近工作区、全局行、旧数据迁移
- 轮次判定 `counts_as_turn`：真提示开一轮；隐藏回忆行与内核标记的中途 steer（`ChatMessage::is_steer`）都属于当前轮，不另计。steer 的识别归内核（含无标记旧行的前缀回退），本层不持有任何文本约定。日志索引与宿主共用这一份定义
- 蒸馏花费侧通道 `SessionMeta.distill`（`DistillUsage`）：由 `add_distill_usage` 在会话写锁内累加。它是本层自有记账——`Writer::meta` 在调用方未给值时沿用旧值，所以轮末 upsert、设置写入、改标题都不会抹掉后台任务刚记的数
- 数据目录由宿主经 `init_data_dir(_with_namespaces)` 注入

## 不做
- 不知道引擎、工具与权限；只存取已序列化的会话记录与线类型
