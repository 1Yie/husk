+++
owns = ["crates/agent-store/**"]
forbidden_deps = ["agent-kernel", "agent-coding", "husk-host", "agent-skills"]
invariants = ["旧 redb 数据只经校验过的快照导入，不加锁、不改动原文件", "单字段修改只经 update_prefs / update_session_links：load+save 会丢并发写者"]
+++

会话持久化：基于文件的 JSONL 日志与原子索引。

关键词：session persistence, jsonl journal, session index, migration, recent workspaces, workspace prefs, global rows, distill usage, bg job record, tool results, tool rows, linked roots, locked read-modify-write

## 职责
- `SessionStore` / `SessionDatabase`：每会话一份 JSONL 日志，锁只覆盖单次文件事务
- 会话索引与 meta、每工作区偏好行、最近工作区、全局行、旧数据迁移
- 偏好与关联许可的写入口一律是**带锁读改写**：`update_prefs(f)` 与 `update_session_links(id, f)` 在各自的文件锁里完成「读 → 改 → 原子写」。`load_prefs` + `save_prefs` 只留给整份替换（迁移、测试夹具）；用它们做单字段修改会丢掉并发写者的改动——模型 pin、设置页、一次关联授权写的是同一行
- 轮次判定 `counts_as_turn`：真提示开一轮；隐藏回忆行与内核标记的中途 steer（`ChatMessage::is_steer`）都属于当前轮，不另计。steer 的识别归内核（含无标记旧行的前缀回退），本层不持有任何文本约定。日志索引与宿主共用这一份定义
- 索引里的调用关联：assistant 行记下自己发起的 `(call id, 工具名)`，tool 行记下工具名与发起它的行号。`tool_rows(id, tools)` 据此只读匹配的结果行和发起行，开销随结果数而非会话长度增长，每行带工具名、调用参数、结果正文、`is_error` 与它在可见历史里的行号；`tool_results(id, tools)` 是它去掉失败行后的旧形状。索引规则（`TURN_RULE`）变化时旧缓存从日志重建一次
- 蒸馏花费侧通道 `SessionMeta.distill`（`DistillUsage`）：由 `add_distill_usage` 在会话写锁内累加。它是本层自有记账——`Writer::meta` 在调用方未给值时沿用旧值，所以轮末 upsert、设置写入、改标题都不会抹掉后台任务刚记的数
- 后台任务记录 `save_bg_jobs` / `load_bg_jobs`：与 journal 同目录的 `{id}.bg.json`，原子写，删除会话时一并删除。载荷对本层不透明——形状由宿主定义，本层只负责存取；任务的进程本身不在持久化范围内
- 关联工作区两层存取：持久许可是 `WorkspacePrefs.linked_roots`（`LinkedRootPref { path, access }`，随主工作区分区）；本会话许可是与 journal 同目录的 `{id}.links.json`（`save_session_links` / `load_session_links` / `update_session_links`），原子写，删除会话时一并删除，所以重启应用后本会话许可仍在
- 数据目录由宿主经 `init_data_dir(_with_namespaces)` 注入

## 不做
- 不知道引擎、工具与权限；只存取已序列化的会话记录与线类型。`tool_rows` / `tool_results` 只按调用者给的工具名筛选，不认识任何具体工具
- 不校验关联路径是否合法：校验属于宿主
- 不合并两层许可：谁该赢（工作区那份更宽）是宿主的规则，本层原样保存两份

## 对外契约
- `SessionStore::{load_prefs, save_prefs, update_prefs}`、`{load_session_links, save_session_links, update_session_links}`：改一个字段用 `update_*`，它持锁完成读改写；`save_*` 是整份替换
- `SessionStore::tool_rows(id, tools) -> Vec<ToolRow>`：`ToolRow { tool, args, content, is_error, index }`，`index` 与 `load_history_page` 切页用的是同一套可见历史行号
