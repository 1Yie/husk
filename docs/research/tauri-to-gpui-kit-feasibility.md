# Husk：Tauri → GPUI + GPUI-Kit 可行性评估

## 评估口径

- 评估日期：2026-10-29。
- 本地依据：`35e3511b`（`chore(release): 0.3.13`）基础上的**当前工作树**，包含已有的终端等未提交改动，不只是该提交。
- 这是静态工程审计及上游文档/源码研究；没有编译 GPUI、运行应用、进行跨平台验证或性能基准测试。
- 本轮不实施迁移，不修改业务代码。上游专项核验见 [GPUI-Kit 上游研究](gpui-kit-upstream-assessment.md)。

## 1. 判断

**技术上存在清楚的迁移路径，但这是“保留 Rust 核心、重写桌面前端及部分平台宿主”，不是替换一组 React 组件。**

推荐新增并行的 `crates/app-gpui`，先验证高风险功能，再决定是否替代 Tauri。不建议直接删除 `app-tauri` 或承诺在一个短迭代内完整替换。

GPUI 是自绘 UI 框架，GPUI-Kit 是其组件库；不能把组件库的能力等同于 Tauri 的窗口、生命周期、托盘、安装包和发布能力，也不能把 GPU 渲染等同于已经解决输入法、无障碍或性能问题。

**首先需要处理产品兼容门槛：**Kit `0.8.1` 官方安装文档写明 macOS 15+，而本项目当前最低系统版本为 13.4。文档基线不等于本次已测得二进制硬下限，但也没有证据可以保证 13.4 可用。若必须保持现有系统覆盖，先验证兼容路线，不能直接批准完整替换。Kit 上游 CI 也未覆盖本项目的 macOS Intel 目标。

### 上游基线与选型边界

截至本次核验：

| 项目 | 证据与工程含义 |
|---|---|
| 当前发布版 | `gpui-kit 0.8.1`，2026-10-26 发布；固定源码提交 `99576db58ec8103417c41a092e891b4022d24e7e` |
| 项目身份 | 真实仓库 `longbridge/gpui-kit`；历史 `longbridge/gpui-component` 已重定向到该仓库，组件 crate 仍然存在 |
| 默认引擎 | `gpui-pre =0.3.10`，配套 platform/web/macros 同版本；不是默认 `gpui 0.2.2` 或 `gpui-ce` |
| License | Kit/默认 GPUI 快照为 Apache-2.0，可用于 MIT 应用；分发保留上游许可及适用 NOTICE、图标许可等，不等于全部改成 MIT |
| 编译工具链 | 官方当前锁图基线 Rust 1.92+；crate 本身未声明 `rust-version`，并非本次测出的精确 MSRV |
| Linux | 有 X11/Wayland 后端，实际窗口需要可工作的 Vulkan 驱动；不等于无系统依赖或已验证 NVIDIA 组合 |
| Windows | 官方 Windows 10+ / MSVC 路线，存在实际 CI；不应误判成只支持 macOS/Linux |
| 可用基础能力 | Input/Editor、TextView Markdown、虚拟列表、MessageScroller、表格、树、Dock、主题、AccessKit 及测试支持 |
| 仍需应用负责 | 终端模拟器/PTY 视图、完整平台 overlay、系统托盘、更新链和生产打包；不能用组件同名推断全部已有 |

建议先使用 Kit 默认引擎和最小必要 features，保留锁文件，避免同时引入另一套不匹配的 GPUI 类型或一开始就切换可选 GPUI Fast。先对默认引擎测基线，之后再单独评价引擎优化，避免混淆 UI 迁移和渲染器切换的收益。

关键原始来源：[发布 API](https://crates.io/api/v1/crates/gpui-kit/0.8.1)、[安装要求](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/installation.md)、[依赖与引擎来源](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/Cargo.toml)、[许可](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/LICENSE-APACHE)。更完整的来源与验证边界见配套研究文档。

## 2. 当前工程为什么有迁移基础

当前依赖方向大致为：

```text
React / Tailwind / xterm / Markdown / Zustand
                   │ invoke / events / Channel
                   ▼
app-tauri：IPC、桌面宿主、平台集成、终端、部分业务服务
                   │
                   ▼
husk-host：SessionManager、会话 Actor、工作区和共享装配
                   │
                   ▼
agent-llm / agent-store / agent-context
korei / korei-ext：kernel、IPC 契约、工具、sandbox 等
```

重要边界：

1. 根 `Cargo.toml` 的 kernel、IPC、路径和工具等依赖已经来自独立的 `korei` / `korei-ext` 仓库。此次 UI 迁移应消费现有 Rust API，不以修改上游 kernel 为前提。
2. `backend/kernel.rs::KernelState::spawn()` 基本是 `SessionManager::spawn(app_paths())` 的包装，不依赖 WebView。宿主共享并非需要从零设计。
3. `SessionManager` 的事件是 `(workspace_root, session_id, UiEvent)`。新 UI 必须保留工作区和会话的联合身份；只按 session id 路由会串流。
4. `backend/ipc/forwarder.rs` 不只是 `app.emit()`：还维护会话 running/preview、待审批 request id，以及电脑控制授权与 HUD 生命周期。迁移时不能只换成 `cx.notify()`，否则可能破坏实际业务状态和安全语义。
5. `kernel.rs::app_paths()` 明确使用 `AppPaths::from_app_name("husk", ".husk")`，并含旧名称目录迁移。数据兼容不要求保留 Tauri，但要求保留这些路径及持久化格式。

迁移规模不能只数 Tauri endpoint：当前 `generate_handler!` 有 **17 个入口**；其中 `agent_session` 单个入口就分派 **93 个已识别业务 op**（主 match 59、提前处理 29、sidebar 5，按唯一 op 字符串计数，不是 93 个界面）。PTY 另走 Channel，应用事件还有 `agent://event`、更新进度与通知点击。这些大部分业务可复用，但每个 UI 接线和结果状态仍要迁移验收。

源码依据：

- [workspace 依赖](../../Cargo.toml)
- [宿主职责说明](../../.blueprint/husk-host.md)
- [Tauri 状态包装及路径](../../crates/app-tauri/backend/kernel.rs)
- [启动、窗口、托盘及 IPC 注册](../../crates/app-tauri/backend/main.rs)
- [带路由信息的事件转发](../../crates/app-tauri/backend/ipc/forwarder.rs)
- [会话操作分派](../../crates/app-tauri/backend/ipc/sessions.rs)
- [侧栏快照与分页](../../crates/app-tauri/backend/ipc/sidebar.rs)

另外，根 workspace 明确排除的 `crates/app-desktop` 是本地 iced 实验，不是已经存在的 GPUI 实现或可发布原生替代品，不能把它计作迁移已完成的工作。

## 3. 哪些保留，哪些重写

| 层 | 建议 | 注意事项 |
|---|---|---|
| kernel、LLM/provider、工具、sandbox、持久化、检索 | 保留现有 Rust 实现 | 迁移 UI 不应顺带重写 agent 核心 |
| `husk-host` / SessionManager / 会话 Actor | 优先直接复用 | 明确跨线程调用与锁边界，不阻塞 GPUI 主线程 |
| Rust 事件与命令领域类型 | 保留契约 | 去掉 JSON/WebView 传输不等于去掉事件顺序、生命周期与错误处理 |
| app-tauri 内纯业务逻辑 | 有选择地提取到无 UI 依赖模块 | 不要让 app-gpui 依赖含 Tauri 的桌面 bin crate |
| PTY 后端 | 提取并保留可复用部分 | Tauri `Channel`、window label、销毁/退出回收需要新适配 |
| React/TSX、CSS/Tailwind、Radix、Zustand | Rust 重写 | 可复用交互规格、设计 token、数据格式和部分算法思想，不是直接复用源码 |
| xterm.js 终端 | 重写或引入独立原生终端方案 | PTY 不等于终端模拟器；字符状态机、屏幕布局和绘制仍需实现 |
| Markdown、代码块、diff、图片、文件预览 | 按能力差异重写 | 必须逐项验收，不以组件同名认定兼容 |
| 托盘/通知/剪贴板/单实例/更新/覆盖层 | 逐项适配平台层 | 已有非 Tauri 的 Rust 实现可能保留；宿主入口不能原样照搬 |
| 测试 | 留下行为规范，替换 UI 驱动层 | Vitest、DOM、浏览器截图不直接验证 GPUI |

不能用“Rust 后端可复用”推导出“工程大部分工作不用做”。交互行为和平台适配通常比普通表单控件更决定迁移成本。

具体可提取候选是 `backend/terminal.rs`（当前本身不依赖 Tauri）、`ipc/workspace_files.rs` 的路径解析/列表/预览、`ipc/sidebar.rs` 的快照/游标逻辑，以及 updater 的下载/校验/取消协调。`rfd`、`ksni` 和平台通知代码已有非 WebView 的实现基础，不需要把“移除 Tauri”误当成“所有系统能力都从零写”。

### 3.1 前端规模复核

对当前 `crates/app-tauri/frontend/src` 递归统计 `.ts/.tsx`，共 **277 个文件**。按 `__tests__` 目录或 `.test/.spec` 文件名识别测试，排除 `src/test` 的 4 个支撑文件及 `vite-env.d.ts`，得到 **177 个非测试源文件、95 个测试文件、5 个支撑/声明文件**：

| 范围 | 非测试源文件 | 测试文件 | 非测试物理行数 |
|---|---:|---:|---:|
| Chat | 56 | 41 | 19,454 |
| Settings | 28 | 13 | 7,438 |
| Terminal | 15 | 9 | 2,645 |
| Workspace files | 5 | 2 | 491 |
| Update | 3 | 2 | 365 |
| App / components / hooks / lib / stores / types 等共享代码 | 70 | 28 | 9,780 |
| 合计 | 177 | 95 | 40,173 |

测试物理行数为 15,259。物理行数包含注释和空行，不代表有效逻辑行；不包含 CSS、脚本、资源或 Rust，也不证明每个文件都被生产入口使用。统计来自当前工作树即时复核，不是按发布提交冻结的清单，更不能据此线性换算工时。

### 3.2 容易误估的功能边界

- **聊天不是通用列表。** 当前已有变高内容测量/缓存、虚拟化、overscan、底部跟随、历史分页、滚动锚点，以及实时事件与历史重建的一致性。Kit 的虚拟列表和 MessageScroller 可作为基础，但这些应用行为仍须迁移。
- **Markdown 不只是正文。** 当前 Streamdown 接入 CJK、数学、语法高亮、按需加载 Mermaid、图片、外链确认，以及自定义 diff/patch fence。新组件支持 Markdown，并不等于插件和交互逐项等价。
- **文件区域是只读预览，不是完整 IDE。** 当前有文件树、代码/Markdown/图片预览及 diff 展示，不能按迁移 Monaco/CodeMirror、完整编辑器或 Git 客户端来膨胀范围。仍须保留大文件上限、异步请求防陈旧、加载失败和高亮回退行为。
- **设置页的主要成本是行为。** 除通用表单外，还涉及 provider/API key、MCP、instructions、memory、rules、plugins、skills、subagent、appearance、terminal、stats 等配置流程，必须保留验证、掩码、保存/重载和错误态。

源码核查入口：

- [聊天列表](../../crates/app-tauri/frontend/src/features/chat/components/chat-stream/index.tsx)：虚拟化及滚动相关逻辑见 1500–1611、1837–2032、2699–2750 行。
- [实时事件 reducer](../../crates/app-tauri/frontend/src/features/chat/hooks/apply-event.ts) 与 [会话 IPC](../../crates/app-tauri/frontend/src/lib/agent-ipc/sessions.ts)。
- [Markdown 管线](../../crates/app-tauri/frontend/src/features/chat/components/chat-stream/markdown-stream.tsx)：1–13 行插件导入、67–116 行 diff fence、118 行起 Mermaid 按需加载。
- [只读文件预览](../../crates/app-tauri/frontend/src/features/workspace-files/workspace-files.tsx)：64 行只读标识，70 行起大文件/高亮处理，182–250 行请求生命周期。
- [终端会话](../../crates/app-tauri/frontend/src/features/terminal/terminal-session.ts)：74–75 行 Channel，write/resize/close 及其生命周期适配。

### 3.3 测试资产如何留下

95 个测试文件不能全部直接在 GPUI 上运行，但不应丢掉其行为价值：

- 把事件归并/历史重建、分页/测量、终端布局/偏好/render queue/resize/reveal、配置校验和陈旧请求保护等用例，作为 Rust 实现的输入输出规范。
- React/DOM 测试重写为 GPUI 状态/交互测试；现有浏览器滚动、终端与窗口 resize 验收转成原生黑盒场景。
- 仍然区分“算法正确”“UI 正确”和“真实 PTY/显示服务器正确”；浏览器 mock 不证明原生输入法或终端吞吐。

## 4. 必须先证明的高风险点

### 4.1 终端

当前不是简单展示命令输出。至少要验证：

- 中文输入法组合/候选框、中文宽字符、emoji、字体 fallback。
- 前后台切换、隐藏后恢复、窗口缩放、高 DPI、PTY rows/cols 同步。
- 选择/复制/粘贴、快捷键冲突、历史滚动、ANSI 色彩及主题变化。
- 持续高吞吐输出下不阻塞聊天，关闭窗口/退出应用后子进程正确回收。

现有 `portable-pty` 后端可以减少工作，但不能代替 xterm.js 的模拟和渲染。若必须嵌入 WebView 才能保留 xterm，则这是混合方案，不是完整去 WebView；双输入/焦点/渲染体系也会抵消部分迁移收益。

### 4.2 流式聊天与长历史

- 新 UI 必须实现会话切换、历史恢复、工具卡片、审批操作与滚动锚点，而不只是把字符串放到列表。
- 列表虚拟化不自动解决变高 Markdown、代码块、图片加载、流式增量测量和“用户正在看历史时不抢滚动”。
- 可以合并绘制刷新，不能随意丢弃 delta、审批、工具结束或电脑控制撤权事件。
- 事件处理线程不能长期持有全局 `SessionManager` 锁；现有 forwarder 已有避免逐 delta 加锁的注释。
- 首轮验收应包含流式输出时展开 thinking/tool、离开底部加载历史再返回，以及快速切换会话时迟到事件/响应不污染新视图。

### 4.3 电脑控制提示和紧急停止

当前授权期间有独立 glow/HUD，主窗口隐藏后仍须可见；停止操作绑定的是获授权会话，不是当前 UI 活跃会话。

Linux 当前使用 GTK3 layer-shell 处理 Wayland 上的覆盖层摆放（`backend/ipc/layer_shell.rs`、Cargo 平台依赖）。替换成 GPUI 后要实际验证：透明、置顶、穿透、跨显示器、缩放、主窗口隐藏，以及用户随时停止控制。

**该项是功能与安全验收门槛，不宜作为最后的视觉打磨。** 如果原生 GPUI 窗口无法满足，需要验证独立平台 overlay 后端及其事件循环兼容性，或明确接受不再提供该功能；不能静默降级。

### 4.4 宿主和分发

现版有托盘隐藏/恢复、正式版单实例、通知点击定位会话、图片剪贴板、更新安装与重启、应用退出时等待 PTY 清理。移除 Tauri 不会自动得到这些能力。

现有发布不是单一 Linux 可执行文件：

- macOS Apple Silicon。
- macOS Intel。
- Linux x86_64。
- Windows x86_64。

见 [release workflow](../../.github/workflows/release.yml) 和 [Tauri 配置](../../crates/app-tauri/tauri.conf.json)。

更换 UI 不会自动消除 semantic/fastembed 的 ONNX Runtime 约束：现有流水线明确记录 macOS 最低 13.4、Intel macOS 单独构建 ORT，以及 Linux 构建产物 glibc 基线。安装包、动态库收集、签名/公证和升级路径需要独立重建。

## 5. 建议目标结构

```text
app-tauri ── Tauri adapter ─┐
                           ├── husk-host / 必要的共享桌面服务
app-gpui ─── GPUI adapter ──┘                 │
                                            └── 现有 Rust 核心
```

实施约束：

1. `app-gpui` 是新的叶子入口，直接依赖 `husk-host` 和需要的核心 crate，不依赖整个 `app-tauri`。
2. 首先共享已存在的 host；只提取已经确认要被两个入口消费的服务。不要先搭建通用 UI 框架或过度抽象。
3. GPUI 事件循环在主线程；阻塞接收、网络、文件扫描、PTY 和耗时业务留在后台执行器。用明确的消息/任务桥接回 UI，避免在 render/update 中 `recv()` 或等待全局锁。
4. 复用 host 已有 Actor/运行时边界，先验证 Tokio 与 GPUI 异步调度的接合方式，不把 `tauri::async_runtime` 机械替换成 UI executor。
5. 把“宿主领域事件处理”与“向某种 UI 分发”拆开，避免两套 adapter 各自实现审批/HUD状态机。
6. 两个入口共用路径/格式不等于允许随意并发写同一份数据。试验期使用隔离 profile 或副本，明确单实例和多窗口策略。
7. 新 UI 延续 Husk 现有设计、交互和可测量性能目标；GPUI-Kit 的默认外观不能代替现有产品行为。
8. 更新 workspace、`scripts/check-deps.sh` 和 CI 的入口/依赖规则，但保持 GUI/平台依赖在叶子层，不让 GPUI、GTK 或平台窗口对象进入核心库。保留当前 semantic/WASM 等产品 feature 口径，不以关闭既有能力制造包体或启动性能优势。

从安全角度，移除 WebView 也会移除 DOM/CSP/部分 capability 这一组边界；仍须保留路径校验、危险操作审批、外部链接策略和电脑控制撤权。原生 UI 不是不需要这些安全约束的理由。

## 6. 实施顺序与决策门槛

### 阶段 A：有边界的 PoC

只新建原生入口，不删 Tauri。先跑最难的纵切面：

根 workspace 使用 `members = ["crates/*"]`，所以新增目录会自动影响 workspace 命令。建议 PoC 放独立 worktree，并明确其 workspace/CI 隔离策略；不要让未经验证的 GPU/平台依赖意外进入现有所有构建。Rust 编译继续遵守本项目最多 4 jobs、长构建 `nice -n 10` 的限制。

0. 明确 macOS 13.4 是否必须保留；若必须，先证明选定版本/补丁路线及 Intel 目标可用。
1. 实际目标平台的窗口、GPU 后端、输入法、字体和基础剪贴板。
2. 最小真实 PTY 终端，而不是静态 ANSI 示例。
3. 接入真实 `husk-host` 的流式消息、会话切换和工具审批。
4. 真实 Linux Wayland/X11 的电脑控制 HUD/停止流程。

建议压力数据是“一万条历史消息 + 流式响应 + 并发终端输出”；这是计划中的验收场景，不是已验证性能。

**通过条件**：关键能力可实现、许可证和平台依赖可接受、最小场景在目标环境跑通。终端、IME、overlay 任一无法满足现有必须能力，就暂停全量重写。

### 阶段 B：可日常使用的聊天 MVP

迁移会话/工作区导航、输入区、基础 Markdown、流式响应、工具卡片/审批、必要 provider 设置。明确标注尚未迁移的功能，不能以此版本宣布完整替换。

### 阶段 C：补齐兼容与发布

完成终端、文件与 diff、富文本、其余设置、快捷键、通知、托盘、更新和数据兼容。四个发布目标都进入 CI/打包验证后，再进行真实用户灰度和退路验证。

### 阶段 D：切换默认入口

保持一个可回退的 Tauri 版本，使用原数据副本完成往返兼容检查。只有核心场景、平台能力与发布链均通过，才让 GPUI 成为默认入口，随后单独决定何时删除旧实现。

## 7. 工作量和性能预期

工作量只能给工程预算，不能从组件数量或代码行数换算工期。主要变量是 GPUI 熟悉度、终端/overlay 是否能复用现成方案，以及完整平台矩阵和无障碍要求。

在熟悉本工程、有 Rust 桌面经验，且没有遇到必须修改渲染后端的前提下，可先用以下**累计量级**做预算，而不是排期承诺：

| 交付范围 | 粗略预算 |
|---|---|
| 决策 PoC，证明关键纵切面而非完整产品 | 1–2 人周 |
| 可日常使用的聊天 MVP，明确缺少部分终端/设置/平台能力 | 4–8 人周 |
| 接近现版功能和四目标分发的替换版本 | 16–24+ 人周 |

完整替换属于数月级项目；新增原生终端、macOS 旧版本兼容或 GPU/Wayland 问题可能显著超出此范围。并行开发也不能消除平台验证和发布链的关键路径。建议只先批准 PoC 的固定时间预算，再根据证据修正全量迁移估算。

需要记录同一硬件、同一数据和同一功能配置下的：

- 冷启动与首个可交互时间。
- 空闲及长会话的 RSS、GPU 占用。
- 持续流式更新时的输入到显示延迟、滚动帧时间。
- 并发 PTY 输出、历史加载、会话切换时的尾延迟。
- 实际安装包、构建时间和运行时动态依赖。

去掉 WebKit/WebView 和 JS 桥接有明确结构收益，但没有实测就不能承诺“更快、内存降低某个百分比、包体更小”。GPUI 的 GPU/字体/显示服务器依赖和现有 ORT/WASM 等依赖仍需核查。

## 8. 本轮未验证事项

- 本机只读预检查：Fedora 44 KDE、Wayland、x86_64，Rust/Cargo 1.96.1；`pkg-config` 可见 wayland-client、xkbcommon、fontconfig、Vulkan、libsecret，工具链版本满足官方 1.92+ 基线。`xkbcommon-x11` 没有 pkg-config 条目，默认引擎同时启用 X11 时应先核查对应开发包。未安装或更改任何依赖；这些结果不等于 Vulkan 驱动或 GPUI 窗口已通过测试。
- 没有编译或链接 GPUI/GPUI-Kit。
- 没有运行中文 IME、真实 PTY、Wayland overlay、无障碍或性能测试。
- 没有生成/签名/安装任何平台产物。
- 没有对现有未提交业务修改做回归测试；本轮没有改变这些业务代码。

因此，本报告支持“开展受控原型验证”的决策，不构成“完整迁移已被技术验证”的承诺。

## 9. 什么时候值得做

- **值得进入 PoC**：长期目标明确是去 WebView、统一 Rust UI/状态与核心维护，并愿意承担原生终端、平台显示栈和发布工程；接受先验证系统覆盖范围。
- **暂不值得完整重写**：只为修复某个 WebKit 问题、期待未经实测的内存收益，或者同时要求短期完成、保留全部特性及全部旧系统支持。
- **建议本次决策**：批准一个固定预算、明确退出条件的 `app-gpui` 纵切面试验；PoC 结果出来前，继续保留 Tauri 主线及其现有修复。
