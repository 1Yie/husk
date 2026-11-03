+++
owns = ["crates/app-tauri/frontend/**"]
forbidden_deps = ["tauri-backend", "kernel-core", "kernel-tools"]
invariants = ["会话侧栏每项目/归档筛选默认每页30行；折叠不挂载列表，展开按需读取，滚动到该列表页末约200px才续页；无overflow时仅响应向下wheel或PageDown/End滚动意图，不因渲染/轮询/尺寸变化自动耗尽所有页；错误页不自动重试且不阻挡后续项目，一次滚动意图最多推进一个可加载列表", "项目计数、全部全局置顶和跨项目非归档最近6条来自后端全量轻量快照摘要，不能由已加载页推断；活动会话独立恢复，不能用未加载判断不存在或驱逐缓存；成功摘要明确无active且无本地active时结束启动加载并显示welcome，归档仍可展开", "侧栏刷新保留已加载行并按root:id去重；请求单飞且迟到响应按前端代际与后端快照代际丢弃，旧finally不能清除新请求；仅明确sidebar_snapshot_expired允许一次自动恢复，列表等待新generation后废弃旧cursor并只重建已读页预算，连续失效或真实加载错误保留显式重试；续页失败不冻结已加载行刷新，不自动重试失败续页", "侧栏分页的忙碌提示使用约300ms延迟阈值，快速请求不闪烁，不新增toast", "侧栏截断标题仅用于展示；重命名按需读取完整session_title再打开编辑器，不把截断显示值当成完整标题保存", "终端默认和不可用字体的兜底直接使用随包 JetBrains Mono（含 Nerd Mono 图标）；显式系统字体优先于随包字体，最终才是 monospace，不继承应用代码字体；字体选项源于宿主真实 family 列表并优先展示等宽字体，加载失败仍可使用内置字体"]
+++

Husk 桌面端 React 前端：聊天流、会话侧栏、设置与产物面板。

关键词：react, chat stream, plan card, settings, session sidebar, notification center, artifacts dock, background jobs, apply-event, agent-ipc types, changes feed, frontend, linked workspaces, approval actions, command panel, toolbar density, thinking tiers

## 职责

- 渲染 `UiEvent`（`apply-event`）并发出命令；设置页与产物面板
- 前端类型与 `agent-ipc` 的事件契约保持一致
- 无工作区及工作区无活动会话共用 `WorkspaceWelcome`；活跃对话卡按欢迎内容的本地 inline-size container 决定显隐，阈值由 `index.css` 的 `workspace-welcome` 查询定义。问候、主操作和最近入口独立显示；office 展示最近产物，已有会话空问候与设置统计保持各自布局。
- 两个视图源保持同构：`applyEvent`（实时）与 `viewFromHistory`（重开）必须画出同一条流；新增仅存在于事件里的内容时，历史侧要能从持久化的 args/结果重建
- 审批卡多于一问一答：`ApprovalRequested.actions` 非空时，条带渲染的是那些动作（关联本会话 / 关联并记住）+ 拒绝，点击回传 `decideTool(id, true, actionId)`；为空时维持原来的 允许 / 拒绝。动作的 `id` 对前端不透明，不许在界面上解释它
- 改动面板与会话产物是**会话级摘要**，不是页级：打开会话时与分页并行拉 `changes_feed`，用 `applyChangesFeed` 以整份日志重建 `changes` / `artifacts`。`viewFromHistory` 只看到最新一页，单靠它会让重开的长会话少文件（实时视图则是整个进程累积的，所以此前只有在重启后才显形）。当前页已是整份日志时不拉 feed
- 工作区改动按 root 缓存成功快照：初次读取前 `repo:null`，仅明确 `repo:false` 才回退会话改动；失败保留上次成功的文件、仓库判定与时间戳，在 dock（包括零文件工具空态）显示明确失败、初次未读/旧结果可能过期和重试。后续成功清除错误，项目切换只读新 root，迟到结果不得展示为另一项目；这是错误表达修复，不是 Git 扫描阻塞/安全性修复
- `SUMMARY_TOOLS`（`apply-event.ts`）是 feed 工具清单的唯一来源：写工具 + 产物写入/触碰工具，随请求传给后端
- 右侧 dock 是会话产出的唯一侧栏：改动 / 产物 / 后台三个视图；后台视图读 `husk:bg_jobs` 快照，日志按需经 `bg_logs` 拉取，控制经 `bg_control`；输入框上方的任务胶囊与 dock 胶囊并列，点击切到后台视图；任务状态的措辞集中在 `lib/bg-jobs.ts`
- 主回答 `CappedAssistantText` 向 `MemoStreamdown` 提供当前工作区策略；思考、工具、用户文本、计划与更新说明等共享 Markdown 调用方保持各自默认链接策略。`features/workspace-files/file-reference.ts` 解析本地文件引用，`chat-stream/markdown-stream.tsx` 在净化前适配合法目标，并保留默认 sanitize/harden。工作区 root 取自 `ChatPage`，由其私有、非持久的请求携带工作区与会话归属及 activation，沿现有回调链打开 `SessionDock` 的 `WorkspaceFiles`；过期归属由页面过滤，重复激活由预览重新定位。
- 编程与工作（office）模式均可使用只读 files 页，changes 保留编程模式限制，策略由 `dock/state.ts` 与 `dock/workspace-tools.tsx` 管理。`workspace-files/workspace-files.tsx` 复用已有读取及异步代际保护，负责源码行列滚动与高亮、Markdown 数字位置切换源码、无效位置和截断范围提示及现有扩大预览入口；预览边界以该 owner 为准。最终文件系统边界由后端 `ipc/workspace_files.rs` 验证，前端引用转换只负责导航。
- 文件管理的 Markdown「预览」由 `workspace-files/markdown-resources.tsx` 独立渲染。资源以当前文档目录为基准，规范化后仍须位于可信工作区内；文件链接复用已有 reader，数字行列沿用源码定位。普通标题片段打开对应文档并提示标题定位尚未支持，本地 URL 查询参数显示明确的不支持原因。本地图片经受限预览 IPC 读取后端验证的 raster 数据，保留 alt、错误与大小限制；无凭据的绝对 HTTPS 图片通过原生 `<img>` 和 `no-referrer` 加载，SVG 限于图片上下文，失败显示错误；HTTP 与其他未授权来源保持占位。安全 width/height 属性传给图片，按真实比例显示并受预览宽度约束，同段图片自然 inline 排列和换行。文件或 root 切换废弃旧响应。小预览和展开预览使用同一策略，保留 `skipHtml` 以及默认 raw、sanitize、harden。
- Streamdown 本地引用插件将 root（文件预览还包括文档路径）放入标准插件 tuple 的显式选项；processor 缓存按插件名称及选项区分上下文。聊天默认解析规则仍由 `file-reference.ts` 的 `parseWorkspaceFileReference` 管理，文档相对资源使用独立入口。
- 关联工作区只有**一个入口**：composer 权限选择器右侧的「关联工作区」按钮（`lib/linked-roots.ts` 的 `useLinkedRoots` 是唯一数据源，`components/linked-roots` 是唯一行组件）。列表**每次打开时重读**——授权可能来自模型自己调的 `link_workspace`，本界面没有参与那次写入，只挂载时拉一次就会显示成「还没有关联目录」。添加目录与作用域（本会话 / 本工作区）是同一行上的两个控件，不是两个按钮；设置页不再有这一项
- 通知中心（`lib/notifications.ts`）：循环通知投递到铃铛与系统通知；后台任务失败按会话记住已通知的任务 id，每个失败只通知一次，前台会话且窗口在前时交给页面 toast。`SystemMessage` 的非空反馈直接进入共享 toast，不投递收件箱、OS 通知或提示音，也不进入实时或回放的会话投影。
- 提示音（`lib/notification-sound.ts`）：agent loop 停下（回合结束 / 等审批或回答 / 失败）时用 Web Audio 合成一段短音，不受窗口可见性门控，也不依赖系统通知开关；`notification_sound` 偏好独立控制
- 通知点击导航：订阅后按序读取后端保留的点击目标，等待启动会话恢复完成再打开目标项目和会话，处理后确认消费。
- 指令面板与聊天流里的命令标签共用 `composer-bar/commands.ts` 一份清单。`local` 类命令只驱动界面（模型选择器 / 改动 dock / 设置页，经 `lib/settings-bus.ts`），**绝不**作为文本发给内核；`send` 类才交给 `CommandRegistry`
- 输入框底部工具栏按自身宽度分档（`composer-bar/toolbar-density.ts`：宽 / 中 / 窄），不靠元素被动收缩：中档模式按钮转图标，窄档代理模式与权限合并且一个菜单。**只有模型名允许截断**，其余控件一律不压缩

macOS 的 `MainLayout` 以 panel 外框包住透明的 `main-col` 排布层，聊天、右侧 `SessionDock` 和底部 `TerminalPanel` 各自绘制主题一致的不透明圆角工作面。`TitleBar` 是横跨主区的透明统一工具条，顶部与会话侧栏顶栏对齐；外框只在左右和底部留白。圆角取共享尺寸，面板间隙露出同一个外框；具体几何由 `app/layout/main-layout/index.tsx`、`features/chat/pages/chat/index.tsx` 和面板 owner 定义。Dock 的宽度与终端的高度偏好表示可见卡片尺寸，各自的 reveal wrapper 额外承担间隙，关闭时同时收回；拖动本地预览，释放后落盘。终端 reveal 的目标、拖动初值与最大高度都计入同一间隙预算。聊天卡片按可用高度约束内联命令菜单的滚动区，浮动 composer 保持原定位和键盘行为。侧栏开关及面板布局变化保持聊天与 renderer 的节点身份，其他平台继续使用平铺布局。

会话侧栏在各平台均可向两侧调宽，宽度由 `MainLayout` 的窗口私有 CSS 变量保存，导航外层、内层和 `SessionSidebar` 共享该值；收起只将可见宽度归零，展开恢复已提交宽度。最小宽度保留模式按钮与固定操作，macOS 顶栏始终不绘制 Husk 品牌图标、文字，保留原生交通灯占位和模式按钮；其他平台保留品牌。最大宽度受主区可用空间限制，窗口缩小时重新约束，默认值与边界定义在 MainLayout 的 resize owner 中。指针预览按帧合并且保留原输入焦点，释放提交到 DOM，取消、失焦、丢失捕获和卸载恢复已提交值；不写入 layout store 或持久化配置。拖动通过既有 layout resize token 抑制终端中间尺寸适配。左手柄从工具条下方开始，关闭侧栏时隐藏。

`MainLayout` 左侧调宽、`SessionDock` 右侧调宽、`TerminalPanel` 调高及 `SplitHandle` 的 columns/rows 五种入口共用 `index.css` 的 `.layout-resize-grip` 绘制中性胶囊。`aria-orientation` 决定横竖方向，指示位于各自命中区中央，默认隐藏，仅在 hover、真实键盘 focus-visible 或拖动时显示，并遵循 reduced motion。几何、边界、比例与尺寸提交仍由各控制器负责；终端鼠标拖动保留输入焦点，结束、取消、失焦、隐藏和卸载释放拖动状态及 capture。非 Mac 调高命中区与面板内实际可见范围对齐，macOS 使用面板间隙。

背景桥 `app/layout/main-layout/sidebar-glass.ts` 测量整个 MainLayout 根矩形，并将当前主题的 `--husk-panel` 传给原生着色，合并帧更新并在单次在途调用期间保留最新值；根尺寸不变的侧栏开关不更新原生几何。原生准备成功后，`data-sidebar-glass` 打开根节点、布局与侧栏结构表面的透明链；侧栏和外框通过 `--husk-shell-surface` 保留 panel 底色，桌面画面从侧栏及工作面间隙透出，内部顶栏、底栏及项目/产物分组保持透明。业务工作面与卡片保持实色；回退恢复 panel 表面，内嵌几何不变。启动主题、运行时外观及加载层共用壳表面，`WorkspaceLoadingVeil` 隐藏已挂载界面而不卸载会话。卸载恢复当前主题并清理观察器、事件、动画帧和原生视图。真实组件验收入口为 `scripts/terminal-reveal/check.mjs --panels` 和 `scripts/terminal-reveal/sidebar-glass-check.mjs`，原生命令使用 mock IPC。

设置页以同一 WebView 中的覆盖页呈现，工作区保持挂载。macOS 的 `features/settings/pages/index.tsx` 复用 `main-workspace-frame` 作为整页外框，导航与标题条透明，原内容滚动节点绘制实色圆角卡片；卡片四边留白，几何取该组件中的共享 spacing/radius utilities。绝对定位的窗口拖动层与 `WindowControls` 保留。原生材质激活且覆盖页存在时，`index.css` 仅关闭下层 MainLayout 的绘制和指针命中，根矩形、私有侧栏宽度与玻璃 owner 保持。关闭设置恢复工作区，其他平台保持平铺表面。设置导航、表单与共享控件沿用各自 owner，覆盖页和 portal 的层级保持 `App.tsx` 与共享菜单定义；真实组件 mock 验收入口见 `scripts/terminal-reveal/README.md`。

## 状态栏与共享控件

模块、默认可见集合、排序、隐藏与恢复默认由 `components/title-bar/strip.tsx` 定义。工作区栏只展示状态；外观页将同一个 `TitleBar` 放入 `StripPreview` 的仿真窗口，用非零示例数据呈现状态与未显示托盘。拖动在释放时提交一次，Escape 取消；布局保存经 `APPEARANCE_CHANGED` 刷新常驻栏，持久化结构归 `husk-host` 的 `AppearanceSettings`。

详情卡共享 `StatPopoverGroup`，悬浮可替换固定卡，关闭后恢复触发器焦点。共享浮层及浮动胶囊使用 `lib/theme.ts` 的磨砂 surface；思考等级词直接使用 `lib/thinking.ts` 的语义色。composer 外框随换肤立即切色，Todo 内层保留折叠动画。

模型选择器根据 `ModelInfo.configured` 引导空配置到模型设置。活动投影在同一父子范围内按 `call_id` 关联工具行；缺少调用身份的结束事件只匹配已启动的同名行。排队调用可直接以取消结果结束，根工具与嵌套工具的时间线投影都保留 `queued` 状态，由共享工具卡呈现等待态。后台子代理在父回合结束后保持自己的活动状态。

格式化规则及按文件写入、检查的命令分别由前端 `.prettierrc` 与 `package.json` 定义；生成的 lockfile 由包管理器维护格式。

## 不做

- 不含内核行为；只经 Tauri IPC 与后端交互
- 新增摘要解析器时不另立工具清单：把工具加进 `WRITE_TOOLS` / `ARTIFACT_*`，`SUMMARY_TOOLS` 自动跟随
- 不判断某个目录能否关联：拒绝列表在后端，界面只显示后端给的理由
- 不在设置页重复一个会话级入口：许可跟着会话走，配置就放在会话的输入框旁
- 不解释 `action` 的语义：它只被原样回传
- 不在组件里写死颜色：思考强度等语义色走 `index.css` 的 `--husk-*` 令牌，亮暗各一套，取用统一走 `lib/thinking.ts` 这类单点函数
