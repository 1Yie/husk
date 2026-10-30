+++
owns = ["crates/app-tauri/*", "crates/app-tauri/{backend,capabilities,icons,linux}/**"]
forbidden_deps = ["ratatui*", "agent-plugin", "agent-computer", "tauri-frontend"]
invariants = ["侧栏专用sidebar/ sidebar_page IPC默认每页30、最大30；cursor不透明并绑定root、归档筛选与snapshot generation，失效明确拒绝；旧projects/list完整列表接口保持兼容", "侧栏缓存仅在Tauri一处持有轻量会话投影，不持有完整SessionMeta/JournalIndex/history；首读、强制刷新、本地结构修改或工作区集合变化重建，跨进程更新每60秒reconcile；普通轮询只更新live与已加载行，发现新增/删除/归档成员变化立即废弃旧枚举并在摘要重建", "缓存扫描、单项meta读取与非活动store打开在manager锁外，事件转发线程不执行全量IO；只保留一个当前排序快照，旧代际cursor明确失效，不累积历史快照", "侧栏页响应不隐含全量sessions；新前端pick/switch请求使用bounded响应选项，旧调用仍保留原sessions形状；存在性查询独立于分页结果", "侧栏投影/缓存（含live覆盖）的title与preview各最多512个Unicode标量；不改持久meta与旧完整列表；全部global pins数量不设总字节上限，重命名按需session_title单项详情读取完整标题", "终端系统字体 IPC 仅向本地应用 WebView 返回已安装字体的真实 family 与等宽标识，不暴露字体路径/文件内容；系统枚举在阻塞线程执行，不读取原生终端外观配置、不扩大权限或 CSP"]
+++

Husk 桌面端 Tauri 2 后端：内核之上的适配层与打包配置。

关键词：tauri, ipc commands, desktop shell, kernel bridge, tray, system notification, packaging, capabilities, changes feed, linked workspaces, approval action

## 职责
- IPC 命令桥（`backend/ipc`）：把前端调用转成 `UiCommand`、把 `UiEvent` 推给前端
- 审批透传：`decide_tool` 把 `action` 一起送进 `ToolDecision`。判决仍然直写引擎的槽（绕过命令泵），因为卡住的那次调用等的是槽、不是命令队列
- 会话历史分页：`open` / `history_page` 只回最新一页（`HISTORY_PAGE`），长会话靠滚动续读
- 会话变更摘要 `changes_feed`：经 `SessionStore::tool_results` 从**整份**持久日志里取出 `payload.tools` 所列工具的成功结果，连同工具名与调用参数；按索引只读这些行，不读整份日志。工具清单由前端传入——它是解析这些结果的一方，清单只存在一处。改动面板与产物栏必须是会话级摘要，只按页重建会让重开的长会话少文件
- 工作区 Git 改动 `workspace_changes`：仅 `GitError::NotARepo` 返回 `repo:false`；扫描/读取失败以 `workspace_changes_failed: ...` 拒绝 IPC，没有活动工作区也明确拒绝，不能把失败当作空仓库成功结果。成功行与子目录投影保持旧契约；仍调用旧 Git 扫描路径，不保证消除 gix status 阻塞或提升文件读取安全性
- 关联工作区 ops（`linked_roots` / `link_root` / `unlink_root` / `set_link_access` / `pick_link_dir`）：全部经 `SessionManager` 的方法，由它在存储（工作区偏好 + 会话文件）与活 actor 的 `LinkedRoots` 句柄之间保持同步，所以授予对下一次工具调用立即生效；`pick_link_dir` 只跑系统目录选择器、不持内核锁，取消则什么都不改。返回的是两层（`workspace` 持久 / `session` 只限本会话），界面据此区分
- 后台任务操作（`bg_list` / `bg_logs` / `bg_control`）：列表与操作结果都是宿主的完整列表（恢复的记录 + 活任务）；`remove` 先尝试记录条目，未命中再交给活注册表
- 偏好透传：`get_default_prefs` / `set_default_prefs` 承载语义召回开关（`semantic_recall`）；`memory_overview` 另外报 `semantic_ready`（开着但模型还在加载），设置页据此把描述从「下载中」换成「已就绪」
- 窗口、托盘、单实例、系统通知投递（`notify` op）与打包配置
- 系统通知的点击回传：唤起主窗口，把发送时附带的 `target` 经 `husk://notification-click` 交还前端；Linux/Windows 经 `notify-rust` 投递；macOS 11+ 直接使用 `UNUserNotificationCenter`，处理异步权限与持久通知的 delegate 回调，不依赖已弃用的 `NSUserNotificationCenter` 或借用 Terminal 身份。
- 点击队列通过 `peek_notification_click` / `ack_notification_click` 与前端交接，界面就绪前或重挂载期间的点击保留到导航处理完成。
- 构建特性：`default = ["semantic"]`——语义召回是主流程的一部分，默认关闭的是那道开关本身，不是构建；关成本的构建用 `--no-default-features`。库 crate 自己仍保持默认关闭

## 不做
- 不复制内核逻辑；行为变更先落在 kernel
- 不渲染界面（属于 tauri-frontend）
- 不解析 diff：`changes_feed` 只转发行，diff 的解读留给前端的同一套解析器
- 不维护工具清单：哪些工具进 feed 由前端的 `SUMMARY_TOOLS` 决定
- 不实现嵌入：模型与开关都在 agent-context，本层只搬运状态
- 不区分「构建有没有模型」：产品默认带上，界面不为此分叉
- 不自己校验关联目录：拒绝列表在 `husk_host::link_policy`，本层只转达它的理由
- 不丢弃 `action`：少传一个字段，用户点「关联」就退化成「批准」，而没有任何报错
