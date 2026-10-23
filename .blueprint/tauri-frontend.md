+++
owns = ["crates/app-tauri/frontend/**"]
forbidden_deps = ["tauri-backend", "kernel-core", "kernel-tools"]
+++

Husk 桌面端 React 前端：聊天流、会话侧栏、设置与产物面板。

关键词：react, chat stream, plan card, settings, session sidebar, notification center, artifacts dock, background jobs, apply-event, agent-ipc types, changes feed, frontend, linked workspaces, approval actions, command panel, toolbar density, thinking tiers

## 职责
- 渲染 `UiEvent`（`apply-event`）并发出命令；设置页与产物面板
- 前端类型与 `agent-ipc` 的事件契约保持一致
- 两个视图源保持同构：`applyEvent`（实时）与 `viewFromHistory`（重开）必须画出同一条流；新增仅存在于事件里的内容时，历史侧要能从持久化的 args/结果重建
- 审批卡多于一问一答：`ApprovalRequested.actions` 非空时，条带渲染的是那些动作（关联本会话 / 关联并记住）+ 拒绝，点击回传 `decideTool(id, true, actionId)`；为空时维持原来的 允许 / 拒绝。动作的 `id` 对前端不透明，不许在界面上解释它
- 改动面板与会话产物是**会话级摘要**，不是页级：打开会话时与分页并行拉 `changes_feed`，用 `applyChangesFeed` 以整份日志重建 `changes` / `artifacts`。`viewFromHistory` 只看到最新一页，单靠它会让重开的长会话少文件（实时视图则是整个进程累积的，所以此前只有在重启后才显形）。当前页已是整份日志时不拉 feed
- `SUMMARY_TOOLS`（`apply-event.ts`）是 feed 工具清单的唯一来源：写工具 + 产物写入/触碰工具，随请求传给后端
- 右侧 dock 是会话产出的唯一侧栏：改动 / 产物 / 后台三个视图；后台视图读 `husk:bg_jobs` 快照，日志按需经 `bg_logs` 拉取，控制经 `bg_control`；输入框上方的任务胶囊与 dock 胶囊并列，点击切到后台视图；任务状态的措辞集中在 `lib/bg-jobs.ts`
- 关联工作区只有**一个入口**：composer 权限选择器右侧的「关联工作区」按钮（`lib/linked-roots.ts` 的 `useLinkedRoots` 是唯一数据源，`components/linked-roots` 是唯一行组件）。列表**每次打开时重读**——授权可能来自模型自己调的 `link_workspace`，本界面没有参与那次写入，只挂载时拉一次就会显示成「还没有关联目录」。添加目录与作用域（本会话 / 本工作区）是同一行上的两个控件，不是两个按钮；设置页不再有这一项
- 通知中心（`lib/notifications.ts`）：判定哪些事件值得通知，投递到铃铛与系统通知；后台任务失败按会话记住已通知的任务 id，每个失败只通知一次，前台会话且窗口在前时交给页面 toast
- 提示音（`lib/notification-sound.ts`）：agent loop 停下（回合结束 / 等审批或回答 / 失败）时用 Web Audio 合成一段短音，不受窗口可见性门控，也不依赖系统通知开关；`notification_sound` 偏好独立控制
- 通知点击导航：订阅后按序读取后端保留的点击目标，等待启动会话恢复完成再打开目标项目和会话，处理后确认消费。
- 指令面板与聊天流里的命令标签共用 `composer-bar/commands.ts` 一份清单。`local` 类命令只驱动界面（模型选择器 / 改动 dock / 设置页，经 `lib/settings-bus.ts`），**绝不**作为文本发给内核；`send` 类才交给 `CommandRegistry`
- 输入框底部工具栏按自身宽度分档（`composer-bar/toolbar-density.ts`：宽 / 中 / 窄），不靠元素被动收缩：中档模式按钮转图标，窄档代理模式与权限合并且一个菜单。**只有模型名允许截断**，其余控件一律不压缩

## 不做
- 不含内核行为；只经 Tauri IPC 与后端交互
- 新增摘要解析器时不另立工具清单：把工具加进 `WRITE_TOOLS` / `ARTIFACT_*`，`SUMMARY_TOOLS` 自动跟随
- 不判断某个目录能否关联：拒绝列表在后端，界面只显示后端给的理由
- 不在设置页重复一个会话级入口：许可跟着会话走，配置就放在会话的输入框旁
- 不解释 `action` 的语义：它只被原样回传
- 不在组件里写死颜色：思考强度等语义色走 `index.css` 的 `--husk-*` 令牌，亮暗各一套，取用统一走 `lib/thinking.ts` 这类单点函数
