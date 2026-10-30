# GPUI Kit 上游适配评估（仅 primary-source 研究）

- 访问日期：**2026-10-29 UTC**。日期来自本次实际 HTTP Date/本机 UTC；发布日另据 crates.io 元数据，未用访问日代替。
- 范围：官网、docs.rs、crates.io API、这些页面指向的源码仓库及 GitHub Actions 官方记录。**未构建、未运行GUI、未做性能实测、未修改现有用户文件**。下文“明确”代表文档/源码有证据，不代表我们已在设备验证；“未验证”不是证明不支持。
- 稳定版本基线：`gpui-kit 0.8.1`，发布 **2026-10-26T08:36:31.575634Z**；tag `v0.8.1` → `99576db58ec8103417c41a092e891b4022d24e7e`。访问时 main 为 `861070bb50573741e17b6cdbb99c00ff1658f687`（2026-10-29T01:48:06Z），本报告不用 main 新特性冒充已发布能力。crate `.cargo_vcs_info.json` 的打包来源为 `99576db58ec8103417c41a092e891b4022d24e7e`（可能与tag提交不同，均保留）。[版本API](https://crates.io/api/v1/crates/gpui-kit)；[tag API](https://api.github.com/repos/longbridge/gpui-kit/tags?per_page=30)；[发布crate](https://crates.io/api/v1/crates/gpui-kit/0.8.1/download)。

## 1. 结论与针对本项目的门槛

1. **不能按“Tauri框架替换、UI代码保留”估算**：这是原生Rust GPUI组件栈，不是React/DOM渲染层。现有前端和平台插件需要重新建立等价能力；这是架构推断，不是上游提供迁移工具的承诺。[crate说明](https://docs.rs/gpui-kit/0.8.1/gpui_kit/)；[初始化源码](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/crates/kit/src/lib.rs)。
2. **明确的产品兼容阻挡：现有 macOS 13.4 对不上 kit 当前官方 macOS 15+ 安装基线**。未见证据证明0.8.1在13.4可运行；也没有把该安装声明伪装成二进制实测下限。必须先决定是否保留13.4，或另行验证旧版本/补丁路线；本次不实施。[安装要求](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/installation.md)。
3. **不存在已证实的 GPL 商用阻挡或“不支持 Windows”结论**：Kit与默认GPUI发行的声明和许可证都是Apache-2.0；Windows有官方CI实跑。MIT项目可以引入Apache依赖，但不能把其代码改称MIT、删除许可证/NOTICE义务。[Kit完整许可](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/LICENSE-APACHE)；[默认引擎许可](https://docs.rs/crate/gpui-pre/0.3.10/source/LICENSE-APACHE)；[CI实跑](https://github.com/longbridge/gpui-kit/actions/runs/40292368384)。
4. **终端与桌面集成不是组件齐全就解决**：已验证列表、输入/编辑、Markdown、表格、树、dock的文档/源码；**未发现Kit内置PTY/终端模拟渲染器、系统托盘和自动更新服务**。自动更新文档明确交给应用/分发系统。终端、Wayland/NVIDIA及computer-use overlay/HUD是迁移前必须单独验证的关键路径。[组件目录](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/index.md)；[更新边界](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/auto-update.md)。
5. **四目标覆盖不等于四目标已验证**：Kit该tag CI测试实际覆盖Linux x86_64、Windows x86_64、macOS ARM64；**没有找到macOS Intel测试job**。文档/上游平台支持不能填补此缺口；Linux普通CI也不能证明用户的Wayland+NVIDIA组合。[CI配置](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/.github/workflows/ci.yml)；[精确job记录](https://api.github.com/repos/longbridge/gpui-kit/actions/runs/40292368384/jobs?per_page=100)。

## 2. 名称、来源与版本对齐

| 名称 | 已核实身份；不可互换之处 |
|---|---|
| `gpui-kit` / gpui-kit.com | 当前真实仓库是 **longbridge/gpui-kit**；聚合GPUI、无样式Base、Component、默认assets。默认features为`component, assets`，不是只是一套单独控件。[官网](https://gpui-kit.com/)；[Cargo](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/crates/kit/Cargo.toml)。 |
| `longbridge/gpui-component` | GitHub官方API现重定向同一个仓库ID至`longbridge/gpui-kit`；历史项目名不是另一个不相干的Kit。`gpui-component`仍作为styled组件crate发布，0.8.1发布时间2026-10-26T08:34:57.539769Z；不能据历史0.5文档推断当前版本。[重定向API](https://api.github.com/repos/longbridge/gpui-component)；[crate API](https://crates.io/api/v1/crates/gpui-component)。 |
| Zed `gpui` | 官方来自`zed-industries/zed`，官网gpui.rs；crates.io稳定`gpui`当次latest是0.2.2，发布2025-10-22T03:56:29.835700Z。**并非Kit0.8.1实际默认依赖**。[API](https://crates.io/api/v1/crates/gpui)；[官网](https://gpui.rs)。 |
| `gpui-pre` | Kit默认依赖Zed GPUI快照包，精确`=0.3.10`，2026-10-26T00:38:24.157635Z；配套platform/web/macros同精确版本。Kit官方解释：仅为快照发布/依赖改名路径，不是另一套渲染器。**本次未从已下载发布包确认Zed快照精确commit**，不填写猜测hash。[API](https://crates.io/api/v1/crates/gpui-pre)；[精确依赖](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/Cargo.toml)；[发布流水线](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/.github/workflows/release-gpui.yml)；[官方说明](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/installation.md)。 |
| `gpui-fast` | Longbridge的另一可选引擎发行，0.1.12发布2026-10-26T08:19:28.323456Z；Kit通过`gpui-fast` feature成组切换core/platform/web及启用的Kit层，不应自行混合两个GPUI类型宇宙。文档的性能表不是本项目实测。[API](https://crates.io/api/v1/crates/gpui-fast)；[feature定义](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/crates/kit/Cargo.toml)。 |
| `gpui-ce` | 独立community fork，仓库`gpui-ce/gpui-ce`；crate0.3.0发布2026-10-20T22:32:59.729648Z，Rust声明1.95，Apache-2.0。**不是Kit0.8.1默认依赖，未找到官方Kit兼容组合保证**。其平台/功能声明不能移植到Kit。[API](https://crates.io/api/v1/crates/gpui-ce)；[源码](https://github.com/gpui-ce/gpui-ce)。 |

Kit edition为2024、crate未声明`rust-version`；官方安装要求Rust **1.92+**，原因是Linux锁图中的`oo7 0.6.0`声明该MSRV；这不是本次编译测得的Kit最低版本。不应从edition推算MSRV。Kit的`^0.8.1`组件依赖与`=0.3.10`引擎锁定不同，建议评估必须保留应用Cargo.lock，不覆盖单独gpui包。所有版本均为本次API实际返回，而非环境日期外推。[依赖API](https://crates.io/api/v1/crates/gpui-kit/0.8.1/dependencies)。

## 3. License：商用、分发与完整条款

**明确**：Kit、Base、Component及默认gpui-pre声明Apache-2.0，允许商业使用、修改、复制、衍生与分发，**不是GPL，也不要求整个MIT应用开源**。Zed仓库根许可证不能替代GPUI子crate实际许可证；反过来，也不能从GPUI为Apache推断整个Zed均为Apache。[Kit](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/LICENSE-APACHE)；[gpui-pre完整license](https://docs.rs/crate/gpui-pre/0.3.10/source/LICENSE-APACHE)。

Apache 2.0完整条款覆盖：§1定义；§2版权授权（永久、全球、非独占、免许可费、不可撤回）；§3专利授权及提起相关专利诉讼时终止；§4再分发须附许可证、标注修改、保留适用版权/专利/商标/归属声明，存在NOTICE时在允许位置保留相应归属；可另订自己修改的条款但须履行上游义务；§5主动提交贡献的默认许可；§6不授权商标；§7免责；§8责任限制；§9可自行有偿承诺支持/担保但不能让贡献者负责，并承担相应赔偿。**不能把“允许商用”省略成“无分发义务”**。本报告附上仓库完整license文本（含copyright和全部条款），并链接Apache官方全文：https://www.apache.org/licenses/LICENSE-2.0 。

默认assets含Lucide ISC，以及部分Feather派生图标的MIT条款；商业使用也须保留各自copyright/permission notice。[完整图标许可](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/crates/assets/LICENSE-LUCIDE)。仓库另有额外的**CC BY 4.0文档许可**（需署名、来源/license链接和修改说明；不撤销既有Apache授权；不覆盖代码示例、网站实现及无权重新许可的第三方内容；[CC BY 4.0完整法律条款](https://creativecommons.org/licenses/by/4.0/legalcode)），不应将Apache代码许可自动套给文档/网站内容：[LICENSE-DOCS.md](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/LICENSE-DOCS.md)，全文附后。**尚未审计所有传递依赖、字体、grammar、可选webview/语音feature的完整SBOM与NOTICE**；MIT项目最终分发仍需做选定feature和四平台依赖审计。本节是基于文本的工程评估，不替代法律意见。

## 4. 构建/运行与四目标

- **macOS**：文档明确15+及Xcode Command Line Tools；Kit显式启用platform的`font-kit`，文档警告直接依赖GPUI缺此feature会无文字。ARM CI成功不代表Intel或13.4成功；本次不自行声称Xcode具体大版本或Metal导致15+的原因。[安装](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/installation.md)；[默认features](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/Cargo.toml)。
- **Windows**：文档要求10+、Visual Studio 2022 C++ workload/Windows SDK、MSVC Rust，以及PATH可找到CMake；官方x86_64-windows-msvc测试job成功。可选GPUI Fast的Windows manifest属于宿主责任，Kit显式启用`windows-manifest-provided-by-host`；不能因此推断分发签名/安装器已自动生成。[安装](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/installation.md)；[Cargo](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/Cargo.toml)。
- **Linux**：文档要求图形Wayland或X11会话及可工作的Vulkan驱动，只有loader/无头shell不足；默认platform启用`x11, wayland, runtime_shaders`。Ubuntu依赖列表详见原文证据（编译器、pkg-config、字体/X11/Wayland/Vulkan及webview相关库需区分所选feature）。**不再依赖WebView渲染核心UI并不等于无系统图形依赖**。[安装](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/installation.md)；[bootstrap](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/script/bootstrap)。
- **Wayland/NVIDIA**：仅证实后端feature与Vulkan要求；未找到该Kit版本针对NVIDIA驱动、混合显卡、XWayland/原生Wayland双路径的设备验证矩阵。不可把ubuntu CI或Zed可运行当作此项目已通过。需要后续真实硬件验收的结论属于推断。
- **computer-use overlay/HUD**：底层GPUI窗口API存在可配置窗口属性（见附录），但本次未找到Kit跨平台保证全局置顶、点击穿透、无焦点输入、透明截图排除、跨屏DPI/定位、Wayland layer-shell或全局坐标/快捷键/屏幕捕获的完整抽象。**不应把popup/透明窗口直接当作overlay/HUD等价支持**。Wayland compositor策略是需专项实测的风险，而不是已证实Kit全部不支持。[GPUI WindowOptions](https://docs.rs/gpui-pre/0.3.10/gpui/struct.WindowOptions.html)。

## 5. 能力矩阵

| 项目 | 判定与边界 | 精确来源（均访问于2026-10-29） |
|---|---|---|
| 初始化 | **明确**：GPUI application，assets，Kit init，Root/窗口包装；不是Tauri invoke模型。 | [lib.rs](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/crates/kit/src/lib.rs)；[Getting Started](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/getting-started.md) |
| 状态/生命周期 | **明确**：Entity/Context、notify驱动重绘、WeakEntity、订阅和Task拥有权；Task丢弃取消，detach例外；关闭视图并不保证阻塞中的后台代码立刻停止。 | [Task](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/task.md)；[Entity](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/entity.md) |
| Tokio | **明确**：GPUI executor不是Tokio；官方HTTP实例创建独立Tokio Runtime并桥接其任务。现有async Tokio服务可保留独立运行时、经channel把owned结果传UI是**设计推断**，不能在UI线程block_on现有服务。 | [Image/HTTP完整示例](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/image.md)；[Task](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/task.md) |
| 虚拟列表 | **明确**：virtual-list组件有可见区域/尺寸/滚动API；只证明可虚拟化，不证明聊天Markdown任意变高行、高吞吐回流已满足本项目。 | [Virtual List](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/virtual-list.md)；[Message Scroller](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/message-scroller.md) |
| 输入/IME | **明确**：Input/InputState、textarea/editor、底层IME输入协议；不等于所有平台中文/日文候选框、组合输入、选择与撤销已实测。须设备覆盖Windows IME、macOS输入法、Linux ibus/fcitx Wayland/X11。 | [Input](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/input.md)；[Textarea](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/textarea.md)；[GPUI EntityInputHandler](https://docs.rs/gpui-pre/0.3.10/gpui/trait.EntityInputHandler.html) |
| 长文本 | **明确**：large-text示例、编辑器及虚拟列表基础；**未验证**大日志、超长单行、多MB中文、流式Markdown的内存/布局/滚动成本，示例数据量不是本仓库性能测试。 | [large-text](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/examples/large-text/src/main.rs)；[Editor](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/editor.md) |
| Markdown/高亮 | **明确**：TextView Markdown/HTML、stream-markdown及markdown_table示例；Kit有tree-sitter与逐语言feature、syntect选项。默认feature不自动等于包含全部语法。 | [TextView](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/text-view.md)；[features](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/crates/kit/Cargo.toml)；[stream](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/examples/stream-markdown/src/main.rs) |
| 表格/树/dock | **明确**：Table/Tree/Dock文档和源码；数据、排序/加载策略、布局持久化宿主接线仍需应用负责。Dock是应用内部停靠，不是系统dock/tray。 | [Table](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/table.md)；[Tree](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/tree.md)；[Dock](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/dock.md) |
| Terminal/PTY | **未找到可验证的Kit成品证据**：下列限定源码扫描未命中终端模拟器/PTY API。terminal.svg、shell命名、普通编辑器都不能当终端。GPUI Shell为另一运行时能力不意味着PTY终端。必须另评估ANSI/VT、cell grid、PTY、滚屏、选择/IME/粘贴/鼠标协议和GPU渲染。 | [组件catalog](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/index.md)；[crates目录](https://github.com/longbridge/gpui-kit/tree/99576db58ec8103417c41a092e891b4022d24e7e/crates) |
| Accessibility | **明确**：AccessKit语义树、role/name/state/actions与headless断言；官方明确headless通过不证明screen reader播报或焦点视觉。**未验证**VoiceOver/NVDA/Orca全流程与四目标覆盖。 | [Accessibility](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/accessibility.md)；[UI tests](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/crates/kit/tests/ui.rs) |
| 主题 | **明确**：Theme/亮暗/自定义主题和字体相关API；不能直接复用CSS/Tailwind tokens而无需映射。高对比、reduced motion及全组件读屏仍需应用验收。 | [Theme](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/theme.md) |
| 菜单/对话框 | **明确**：Kit PopupMenu/Dialog等自绘组件；底层GPUI有set_menus及文件选择prompt API。两者不能混淆；每个平台原生菜单行为与权限需检查。 | [Menu](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/menu.md)；[Dialog](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/dialog.md)；[GPUI App](https://docs.rs/gpui-pre/0.3.10/gpui/struct.App.html) |
| Tray | **未找到**Kit一体化系统托盘API/四平台验证证据；不将TitleBar、PopupMenu、Dock充当系统托盘。 | [catalog](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/index.md)；[crate public API](https://docs.rs/gpui-kit/0.8.1/gpui_kit/) |
| 更新 | **明确不内置**：文档说Kit不选release、不替换已安装app；self_update仅可选应用依赖示例，签名、完整bundle更新/恢复、安装者归属均由应用负责。 | [Auto Update](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/auto-update.md) |
| 打包 | **明确有指导、不是Tauri bundler等价物**：提供macOS app/DMG、Windows ZIP/Inno Setup、Linux归档/包管理指导；签名、公证、installer、架构产物匹配、运行库和更新链条需分发系统拥有。 | [Packaging](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/packaging.md) |
| 性能 | **没有本项目实测**：官网/文档FPS或GPUI Fast数字仅是上游声明；不据此声称替换后一定更快、更省内存。 | [Rendering Models](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/fps.md) |

## 6. 决策建议与CI实证

**建议：目前只能批准受约束的技术验证，不能据上游功能目录批准整体替换。** 必须先解决macOS 13.4产品底线与官方15+安装基线冲突；确立独立终端/PT​Y渲染路线；验证Wayland/NVIDIA和HUD窗口行为；补macOS Intel及实际读屏/IME验收。Apache-2.0不是MIT项目迁移的许可证否决项，但必须建立第三方许可分发清单。该建议是基于本报告证据的工程判断，不是已完成迁移验证。

### CI实证与未验证事项

固定tag成功CI： https://github.com/longbridge/gpui-kit/actions/runs/40292368384 。另一次同commit CI被取消不能覆盖此次成功记录；以下逐job来自官方API而非“GPUI跨平台”营销。

| Job | 结果 | Runner标签 |
|---|---|---|
| Checks | success | ubuntu-latest |
| GPUI Fast (windows-latest) | success | windows-latest |
| Test (x86_64-windows-msvc, windows-latest) | success | windows-latest |
| GPUI Shell Standard Runtime (x86_64-linux-gnu) | success | ubuntu-latest |
| GPUI Shell Standard Runtime (x86_64-windows-msvc) | success | windows-latest |
| Test (x86_64-linux-gnu, ubuntu-latest) | success | ubuntu-latest |
| GPUI Shell Core | success | ubuntu-latest |
| GPUI Fast (macos-latest) | success | macos-latest |
| WebView on gpui-pre (ubuntu-latest) | success | ubuntu-latest |
| GPUI Fast (WebAssembly) | success | ubuntu-latest |
| GPUI Shell Standard Runtime (aarch64-apple-darwin) | success | macos-latest |
| WebView on gpui-pre (macos-latest) | success | macos-latest |
| Test (aarch64-apple-darwin, macos-latest) | success | macos-latest |
| WebView on gpui-pre (windows-latest) | success | windows-latest |
| GPUI Fast (ubuntu-latest) | success | ubuntu-latest |
| GPUI Shell Components | success | ubuntu-latest |
| No default features | success | ubuntu-latest |
| Lint | success | macos-latest |

本次未启动任何构建，故不把上述上游CI当成本仓库集成测试。未验证：macOS Intel和13.4；Windows原生IME/辅助技术；Wayland/NVIDIA/混合DPI；真实透明/点击穿透HUD；终端吞吐和协议完整性；聊天流式富文本回流；全依赖分发license；自动更新与签名链。建议将这些设为批准迁移前的验收门槛，而非迁移后再补。

## 7. 可复核的原文定位（固定tag；所有条目访问于2026-10-29）

以下为所读官方文件的实际匹配行，避免把“存在文档”误写成“已测试可用”。版本模板如`{{gpui_pre_version}}`以同tag Cargo.toml中`=0.3.10`为准。网页可能继续更新，优先使用固定commit源。

### 安装与平台要求

来源：[website/docs/installation.md](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/installation.md)

```text
L3: description: Install GPUI Kit and the platform dependencies required to build Rust desktop applications on macOS, Windows, and Linux.
L9: Install the native toolchain for your operating system, then add the `gpui-kit` crate to a Rust application. The steps below are for desktop development; [WebAssembly](./webassembly) and [Mobile](./mobile) have separate target setup.
L24:       <ul><li>macOS 15 or later</li><li>Xcode Command Line Tools, installed with <code>xcode-select --install</code>. Run <code>xcode-select -p</code> afterward; it should print the selected developer directory.</li></ul>
L27:       <ul><li>Windows 10 or later</li><li>Visual Studio 2022 Build Tools or Community with the <strong>Desktop development with C++</strong> workload, including MSVC and a Windows SDK. Use the MSVC Rust toolchain, not the GNU toolchain.</li><li>CMake available on <code>PATH</code>; check with <code>cmake --version</code> in the terminal where you will run Cargo.</li></ul>
L31:       <pre><code class="language-bash">sudo apt update
L32: sudo apt install -y gcc g++ clang libfontconfig-dev libwayland-dev \
L33:   libwebkit2gtk-4.1-dev libxkbcommon-x11-dev libx11-xcb-dev \
L34:   libssl-dev libzstd-dev libasound2-dev vulkan-validationlayers libvulkan1</code></pre>
L35:       <p>This matches the repository's <code>script/install-linux.sh</code> for Ubuntu 24.04. Other distributions need equivalent development packages. <code>libasound2-dev</code> (ALSA) is needed only by the <code>speech</code> feature. To display a window, run in a graphical Wayland or X11 session with a working Vulkan driver; installing <code>libvulkan1</code> alone does not install a GPU driver.</p>
L40: ## Rust and Cargo
L42: Install Rust and Cargo with the [official Rust installer](https://rust-lang.org/tools/install/). Use Rust 1.92 or later for this repository's current locked dependency graph: its Linux GPUI platform dependency includes `oo7 0.6.0`, which declares Rust 1.92 as its minimum. Then check the tools in the same terminal that will build the app:
L45: rustc --version
L49: Both commands should print a version. On Windows, <code>rustup show active-toolchain</code> should identify an <code>msvc</code> target. If a terminal cannot find Cargo immediately after installation, open a new terminal and run the checks again.
L98: | `cargo` or `rustc` is not found. | Install Rust with rustup, open a new terminal, and rerun the version checks above. |
L99: | A build reports an unsupported Rust version. | Check `rustc --version`; update the active Rust toolchain. A dependency may require a newer compiler than this guide's baseline. |
L100: | Windows reports `link.exe` missing or cannot find a Windows SDK. | Confirm the Visual Studio C++ workload and SDK are installed, then build with an MSVC Rust toolchain from a Visual Studio Developer PowerShell if needed. |
L101: | Linux reports a missing `pkg-config` executable, X11, Wayland, fontconfig, or WebKit header. | Install the Ubuntu packages above, or their equivalents for your distribution. If `pkg-config` itself is missing, install the `pkg-config` package too. The error identifies the missing tool or system library. |
L102: | The program compiles but no window appears on Linux. | Check that the process is running in a graphical Wayland or X11 session and that a Vulkan driver works for that session. A headless shell or Vulkan loader without a driver is insufficient. |
L103: | A window opens on macOS, but no text is rendered. | If you depend on GPUI directly, enable `font-kit` on `gpui-pre-platform` and rebuild. `gpui-kit` already enables it. See [Getting Started](./getting-started.md#macos-text-rendering-font-kit) for the manifest and feature check. |
L110: Rust Debug builds leave GPUI, the component library, layout, and text rendering
L121: Cargo's [package profile overrides](https://doc.rust-lang.org/cargo/reference/profiles.html#overrides) only take effect in the root `Cargo.toml` of your application
L131: rustybuzz = { opt-level = 3 }
```

### 初始化与窗口生命周期

来源：[crates/kit/src/lib.rs](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/crates/kit/src/lib.rs)

```text
L35: //!         gpui_kit::init(cx);
L36: //!         gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Hello))
L86: // Keep GPUI APIs available through the Kit root and preserve the published
L101: pub use ::gpui::*;
L103: pub use ::gpui_fast::*;
L107: pub use gpui_macros::derive_inspector_reflection;
L109: pub use gpui_macros::{
L117: pub use crate as gpui;
L122: pub const GPUI_FAST: bool = cfg!(feature = "gpui-fast");
L130: pub use ::gpui_base as base;
L135: pub use ::gpui_fast_platform as platform;
L137: pub use ::gpui_fast_web as web;
L142: pub use ::gpui_platform as platform;
L144: pub use ::gpui_web as web;
L145: pub use gpui_base::is_mobile;
L163: ///         gpui_kit::init(cx);
L164: ///         gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Hello))
L170: pub use ::gpui_component as component;
L173: pub use ::gpui_kit_assets as assets;
L175: /// Open a window with a Base Root and return the window and application content.
L178: /// The builder returns application content, not another Root.
L181: pub fn open_window<V: Render>(
L187:     let window = cx.open_window(options, |window, cx| {
L190:         cx.new(|cx| base::Root::new(view, window, cx))
L194:         built.expect("open_window ran its build closure"),
```

### 状态、异步与取消

来源：[website/docs/task.md](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/task.md)

```text
L9: In GPUI, a [`Task<T>`](https://docs.rs/gpui-pre/{{gpui_pre_version}}/gpui/struct.Task.html) is the handle to work scheduled by a GPUI executor. Its most important property is **ownership**: dropping the handle cancels unfinished work. A task runs only while its handle is stored, awaited, or explicitly detached. This makes the task's lifetime part of the View's state design, not just a detail of Rust's `Future` trait.
L11: The **spawn API** chooses where work runs; the returned `Task` controls its lifetime. A foreground task can re-enter GPUI through an async [Context](./context) and update an [Entity]. A background task runs away from the UI thread and returns owned data; it cannot mutate Entity state there.
L15: | `cx.spawn(...)` in `Context<T>` | Foreground thread | `WeakEntity<T>`, `&mut AsyncApp` | Awaiting I/O, timers, then updating an Entity |
L16: | `cx.spawn_in(window, ...)` | Foreground thread | `WeakEntity<T>`, `&mut AsyncWindowContext` | Work whose completion needs the same [Window](./window) |
L17: | `cx.spawn(...)` in `App` | Foreground thread | `&mut AsyncApp` | Application-level work without a current Entity |
L18: | `cx.background_spawn(...)` | Background executor | No GPUI context | Expensive parsing or computation on owned `Send` data |
L28: The window opens with a **Replay** button and a **Fade in streamed text** switch. Click Replay: the text clears, then appears in chunks. Click it again before the stream finishes to start a new replay; only chunks tagged with the current replay ID are displayed. Closing the window drops the View's task handles, but a producer already running its synchronous loop can finish that loop before stopping. The complete source is [`examples/stream-markdown/src/main.rs`](https://github.com/longbridge/gpui-kit/blob/main/examples/stream-markdown/src/main.rs); its package and dependencies are in [`examples/stream-markdown/Cargo.toml`](https://github.com/longbridge/gpui-kit/blob/main/examples/stream-markdown/Cargo.toml). This is a runnable workspace example; for a one-dependency app skeleton, start wit
L30: Follow one click through the source: `main` creates the window; `Example::new` creates the channel and saves a foreground receiver `Task`; the button calls `Example::replay`, which increments `replay_id`, clears the text and replaces the background producer `Task`; the receiver checks that ID before updating `TextViewState`. The example uses a worker to simulate incoming chunks. In an actual network stream, await the source's asynchronous read and await a bounded channel send so a slow UI cannot cause unbounded queued data.
L32: ## Build a task you can start, cancel, and fail
L51:         self.task = None; // Drop the previous handle before starting again.
L55:         self.task = Some(cx.spawn(async move |this, cx| {
L57:                 .background_spawn(async move {
L70:                     return; // A newer start or cancel owns the displayed state.
L81:     fn cancel(&mut self, cx: &mut Context<Self>) {
L83:         self.task = None; // Dropping the handle prevents future polls.
L84:         self.status = "Cancelled".into();
```

### Tokio/reqwest桥接

来源：[website/docs/image.md](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/image.md)

```text
L219: tokio = { version = "1", features = ["rt-multi-thread"] }
L233: /// reqwest needs a tokio runtime; GPUI's executors are not one.
L234: static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
L235:     tokio::runtime::Builder::new_multi_thread()
L239:         .expect("failed to start the HTTP runtime")
L296:             let response = RUNTIME.spawn(request.send()).await??;
```

### 虚拟列表

来源：[website/component/virtual-list.md](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/virtual-list.md)

```text
L2: title: VirtualList
L3: description: High-performance virtualized list component for rendering large datasets with variable item sizes.
L6: # VirtualList
L8: VirtualList is a high-performance component designed for efficiently rendering large datasets by only rendering visible items. Unlike uniform lists, VirtualList supports variable item sizes, making it perfect for complex layouts like tables with different row heights or dynamic content.
L14:     v_virtual_list, h_virtual_list, VirtualListScrollHandle,
L15:     scroll::{Scrollbar, ScrollbarState, ScrollbarAxis},
L18: use gpui_kit::{px, size, ScrollStrategy, Size, Pixels};
L23: ### Basic Vertical Virtual List
L27: use gpui_kit::{px, size, Size, Pixels};
L31:     item_sizes: Rc<Vec<Size<Pixels>>>,
L32:     scroll_handle: VirtualListScrollHandle,
L38:         let item_sizes = Rc::new(items.iter().map(|_| size(px(200.), px(30.))).collect());
L42:             item_sizes,
L43:             scroll_handle: VirtualListScrollHandle::new(),
```

### 输入与IME

来源：[website/component/input.md](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/input.md)

```text
L2: title: Input
L3: description: Text input component with validation, masking, and various features.
L6: # Input
L8: For multiple addons, shared frames, and textarea toolbars, see [Input Group](./input-group.md).
L10: A single-line text input with validation, masking, prefix/suffix elements, and
L11: different visual states. Use [Textarea](./textarea.md) for ordinary multi-line
L17: use gpui_kit::component::input::{Input, InputState};
L22: ### Basic Input
L25: let input = cx.new(|cx| InputState::new(window, cx));
L27: Input::new(&input)
L33: let input = cx.new(|cx|
L34:     InputState::new(window, cx)
L38: Input::new(&input)
L44: let input = cx.new(|cx|
```

### 多行输入

来源：[website/component/textarea.md](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/textarea.md)

```text
L3: description: Multi-line text input with fixed rows, soft wrapping, and auto-grow.
L8: `Textarea` is the styled control for ordinary multi-line text. Use
L9: [`Input`](./input.md) for a single line and [`Editor`](./editor.md) for source
L15: use gpui_kit::component::input::{Textarea, TextareaState};
L46: For host-controlled sizing, read `notes.read(cx).display_map().wrap_row_count()`.
L47: The same [prewrap pattern as Editor](./editor.md#sizing-to-content) prepares the
L70: by lines and do not bounce. The frame stays put. It is on by default on iOS, Android and macOS, whose own scroll views bounce, and off elsewhere. When the system asks for reduced motion, the text
L82: cx.subscribe(&notes, |this, state, event: &InputEvent, cx| {
L83:     if matches!(event, InputEvent::Change) {
L90: `insert`, `replace`, `cursor_position`, `soft_wrap`, `searchable`, and
```

### 代码编辑与高亮

来源：[website/component/editor.md](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/editor.md)

```text
L3: description: Source-code editor with syntax highlighting, gutter, folding, and decorations.
L17: ## Language editing rules
L19: `LanguageConfig` describes a language; `.auto_close(bool)` and `.smart_indent(bool)`
L20: are independent editor preferences. Changing languages or replacing rules does
L27:     AutoClosingPair, BracketPair, language_config::LanguageConfig, SyntaxContext, set_language_config,
L30: let rules = LanguageConfig::default()
L40: set_language_config("rust", rules, cx);
L44:         .language("rust")
L50: `set_language_config` replaces the configuration for a language in the current
L53: `pyi` refer to the same language even without its grammar feature. Custom
L55: take precedence over built-in aliases and retain their original case. Unknown languages use
L56: `LanguageConfig::default()`.
L58: Component installs a `LanguageProvider` for language names, editing defaults,
L59: and editor-owned syntax providers. Syntax selection follows the language on the
L60: first edit and after language changes, independently of rendering. Base clients
L61: can install their own service with `set_language_provider`; ordinary Component
```

### 富文本与Markdown

来源：[website/component/text-view.md](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/text-view.md)

```text
L3: description: Renders Markdown and HTML text with optional custom Markdown plugins.
L8: `TextView` renders formatted text in GPUI. It supports Markdown and simple HTML, text selection, code block actions, and custom Markdown plugins for project-specific syntax.
L10: The canonical implementation now lives in `gpui-base`; this module remains a compatibility re-export and provides component-theme adaptation. Base-only setup, complete default styling, and opt-in syntax highlighting are documented on [GPUI Base TextView](../base/text-view.md).
L12: TextView is selectable by default and uses the shared window selection engine from `gpui-base`. Use `.selectable(false)` only when selection must be disabled. See [GPUI Base Text Selection](../base/text-selection.md) when integrating plain text or a custom renderer with the same selection.
L17: use gpui_kit::component::text::{markdown, TextView};
L22: ### Markdown
L24: Use the `markdown` helper when you only need to render Markdown text:
L27: use gpui_kit::component::text::markdown;
L29: markdown("# Hello\n\nThis is **Markdown**.")
L33: You can also construct a `TextView` directly when you need a stable id:
L38: TextView::markdown("preview", markdown_source)
L41: ### HTML
L44: TextView::html("html-preview", "<strong>Hello</strong>")
L51: [Bubble](bubble.md), links, muted text, code and table backgrounds, borders and
L52: selection are derived from that color as well, and syntax highlighting is left
L56: Bubble::new().child(TextView::markdown("reply", text))
```

### 大文本示例

来源：[examples/large-text/src/main.rs](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/examples/large-text/src/main.rs)

```text
L6:     input::{self, Input, InputEvent, InputState, TabSize, Textarea, TextareaState},
L12:     editor: Entity<TextareaState>,
L13:     go_to_line_state: Entity<InputState>,
L19:     pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
L20:         // 10K lines
L21:         let text = "这是一个中文演示段落，用于展示更多的 [Markdown GFM] 内容。您可以在此尝试使用使用**粗体**、*斜体*和`代码`等样式。これは日本語のデモ段落です。Markdown の多言語サポートを示すためのテキストが含まれています。例えば、、**ボールド**、_イタリック_、および`コード`のスタイルなどを試すことができます。\n".repeat(10000);
L24:             TextareaState::new(window, cx)
L31:                 .default_value(text)
L33:         let go_to_line_state = cx.new(|cx| InputState::new(window, cx));
L41:             go_to_line_state,
```

### 表格

来源：[website/component/table.md](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/table.md)

```text
L8: A simple, stateless, composable table component for rendering tabular data. Unlike [DataTable], this component does not include virtual scrolling, sorting, or column management — it is designed for straightforward data display using a declarative API.
L15:     TableRow, TableHead, TableCell, TableCaption,
L26:         TableRow::new()
L32:         .child(TableRow::new()
L36:         .child(TableRow::new()
L49:         TableRow::new()
L55:         .child(TableRow::new()
L61:         TableRow::new()
L68: ### Column Widths
L70: Use `.w()` on `TableHead` and `TableCell` to set fixed column widths:
L73: TableRow::new()
L106: // Custom row hover
```

### 树

来源：[website/component/tree.md](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/tree.md)

```text
L2: title: Tree
L3: description: A hierarchical tree view component for displaying and navigating tree-structured data.
L6: # Tree
L8: A versatile tree component for displaying hierarchical data with expand/collapse functionality, keyboard navigation, and custom item rendering. Perfect for file explorers, navigation menus, or any nested data structure.
L13: use gpui_kit::component::tree::{tree, TreeState, TreeItem, TreeEntry};
L18: ### Basic Tree
L21: // Create tree state
L22: let tree_state = cx.new(|cx| {
L23:     TreeState::new(cx).items(vec![
L24:         TreeItem::new("src", "src")
```

### Dock

来源：[website/component/dock.md](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/dock.md)

```text
L3: description: Production-ready dock layouts with styled tabs, split panes, edge docks, and persistent state.
L9: Dock builds application workspaces from draggable tab groups, nested splits, and collapsible left, right, and bottom docks. It is the layout foundation used by Longbridge in production, not an isolated UI demo.
L11: `gpui-base` owns the data model, layout calculation, and drag-and-drop behavior. `gpui-component` supplies the polished controls and visual language. Use `gpui_kit::component::dock` when you want a Dock ready to fit into a real application.
L20: use gpui_kit::component::dock::{DockArea, DockSkin};
L23:     dock_area: Entity<DockArea>,
L37: The optional version belongs to your saved layout schema. Increase it when your application can no longer restore an older layout.
L39: ## Define a panel
L41: A styled Dock panel implements `BasePanel` for identity and persistence, and `Panel` for its title, tab, and toolbar presentation.
L44: use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
L46: struct FilesPanel {
L50: impl EventEmitter<PanelEvent> for FilesPanel {}
L52: impl Focusable for FilesPanel {
```

### 无障碍

来源：[website/docs/accessibility.md](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/accessibility.md)

```text
L3: description: Build and test accessible GPUI Kit interfaces with AccessKit semantics and actions.
L9: GPUI sends an accessibility tree to platform assistive technology through [AccessKit](https://accesskit.dev/). A useful control has a **role** (what it is), a **name** (how a person identifies it), **state** (such as value, checked, selected, or expanded), and **actions** (what assistive technology can request). Its stable element ID preserves identity across frames; it is not the name a person hears. The same interface must work with a keyboard and show where focus is. Tree properties alone do not implement keyboard behavior.
L11: Applications normally use GPUI Kit's styled components from `gpui_kit::component`. Their interaction comes from the unstyled `gpui_kit::base` layer. Both are available through one `gpui-kit` dependency. Use a standard control before composing a new interactive `div`: it already coordinates pointer input, keyboard input, focus, state, and AccessKit semantics.
L13: For a custom keyboard target, follow [Focus](./focus) first: `track_focus`, Tab order, and an accessible role solve different parts of the interaction.
L21: | Semantics | The controls expose a role, name, relevant state, and available actions through AccessKit. | A painted-frame snapshot contains the expected properties. |
L22: | Operation | Pointer, keyboard, and assistive technology reach the same command. | Save works with each input method; focus moves visibly and predictably. |
L24: Start with the first two steps, then inspect the semantic tree and operate the real window. A passing headless tree assertion cannot prove what a screen reader announces or whether a focus ring is visible.
L34: The test opens a headless window, enters a name, activates **Save**, and checks both application state and accessibility properties written by the rendered GPUI elements. This gives you a concrete path from the model to a visible control and its AccessKit node. For setup of your own test crate, continue with [Testing](./test).
L89: The repository test also checks input focus, status geometry, and `profile.read(cx).submitted`. Checking the model prevents a test from passing when only the label changes. `window.click` exercises pointer activation here; the `backspace` step removes the accented `é` after the pointer has focused the input, testing Unicode editing rather than keyboard navigation to **Save**. As an experiment, remove `.aria_label("Profile name")` and rerun the test: the label assertion shows why visible proximity to other text cannot be assumed to name the input.
L91: To check the keyboard path in a real window, put this complete version of the same view in the existing `examples/hello_world/src/main.rs`, then run `cargo run -p hello_world` from the workspace root. This is a local exercise; restore the example file when finished. The headless test above does not run this keyboard sequence.
L155: The window starts with **Not saved**. Starting before the form, Tab to **Profile name** and confirm a visible focus indication; type `Ada José` and use Backspace once, leaving `Ada Jos`. Tab to **Save**, confirm its focus indication, then press Enter. The visible result should become **Saved: Ada Jos**. While **Save** remains focused, press Space to activate it again. Use Shift+Tab to return to the input, then Tab back to **Save** to verify reverse and forward navigation. If either activation key does not work, inspect the button's focus and key handling instead of treating the pointer test as evidence. Finally, activate **Save** through your target platform's assistive technology and check the announced role, name, and result; the status role alone does not guarantee a live announcement.
L183:         // Invoke the same application command used by its keyboard route.
L206: The explicit label is the announced name. `.test_support()` lets the later integration test find this custom `div` when `test-support` is enabled; it adds no layout container and is inert in normal builds. `Role::GenericContainer` is filtered from the accessibility tree; use an actual role for a meaningful node. `accessibility_id(...)` is a separate, author-provided identifier exposed to platform automation. It maps to identifiers such as UIA `AutomationId` on Windows and `AXIdentifier` on macOS, with Linux AT-SPI support depending on the deployed adapter. It is not a substitute for GPUI's `.id(...)` or for a human-readable name.
L212: GPUI's current `div` API has no general `.aria_disabled(...)` builder. Base controls such as Button and Checkbox gate focus and activation when disabled, but that does not guarantee a native disabled property for every node. Verify both the available tree state and the actual disabled behavior. Likewise, `.track_focus(...)` gives a node a focus path and advertises the accessible Focus action; a role or `.focusable()` alone does not implement a useful keyboard command.
L214: The element tree establishes parent/child relationships. Composite controls may keep keyboard focus on a parent and mark the current child with `.aria_active_descendant()`. GPUI applies that child-side marker only when an ancestor actually has focus. The child needs its own ID and role. This is specialized composite behavior; use the built-in Select, menu, or list behavior when it fits.
L218: ## Accessible actions and keyboard input
L220: An AccessKit action is distinct from a GPUI [Action](./action) dispatched by a keybinding or menu. GPUI exposes it as `AccessibleAction`. For an identified `div`, `.on_a11y_action(action, handler)` registers one requested action; the handler receives optional `ActionData`, `&mut Window`, and `&mut App`. `.on_click(...)` already advertises accessible Click activation and routes it to the click handler, so a second Click handler can perform the command twice. A custom slider, for example, must offer Increment and Decrement as well as its pointer and keyboard controls, and update its numeric accessibility value after the model changes. GPUI Kit's Slider already does this.
L222: Keep the interaction promise consistent: the visible label, accessible name, shortcut, pointer behavior, keyboard behavior, and assistive action should all perform the same command. A clickable painted shape with a hitbox is still missing semantics and keyboard operation until those are implemented. After dialogs or sheets close, restore focus to their trigger. Keep focus visible and ordered according to the task.
```

### 主题

来源：[website/component/theme.md](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/theme.md)

```text
L2: title: Theme
L3: description: Customize colors, typography, radii, and light or dark appearance with the GPUI Component theme system.
L7: # Theme
L9: All components support theming through the built-in Theme system, the [ActiveTheme] trait provides access to the current theme colors:
L12: use gpui_kit::component::{ActiveTheme as _};
L14: // Access theme colors in your components
L15: cx.theme().primary
L16: cx.theme().background
L17: cx.theme().foreground
L20: So if you want use the colors from the current theme, you should keep your component or view have [App] context.
L24: Theme color values remain backward compatible with the existing string format:
L26: ```json
L36: ```json
L45: Top-level theme fields, such as `cx.theme().button_primary`, remain solid `Hsla` values for compatibility. Code that needs the full resolved token can use `cx.theme().tokens.button_primary`; its `.color` field is the solid representative color, and its `.background` field contains the configured `Background`, including gradients.
```

### 菜单

来源：[website/component/menu.md](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/menu.md)

```text
L2: title: Menu
L3: description: Context menus and popup menus with support for icons, shortcuts, submenus, and various menu item types.
L6: # PopupMenu
L8: The Menu component provides both context menus (right-click menus) and popup menus with comprehensive features including icons, keyboard shortcuts, submenus, separators, checkable items, and custom elements. Built with accessibility and keyboard navigation in mind.
L14:     menu::{PopupMenu, PopupMenuItem, ContextMenuExt, DropdownMenu},
L22: ### ContextMenu
L24: Context menus appear when right-clicking on an element:
L27: use gpui_kit::component::menu::ContextMenuExt;
L32:     .context_menu(|menu, window, cx| {
L33:         menu.menu("Copy", Box::new(Copy))
```

### 对话框

来源：[website/component/dialog.md](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/component/dialog.md)

```text
L2: title: Dialog
L3: description: A dialog dialog for displaying content in a layer above the app.
L6: # Dialog
L8: Dialog component for creating dialogs, confirmations, and alerts. Supports overlay, keyboard shortcuts, and various customizations.
L13: use gpui_kit::component::dialog::DialogButtonProps;
L19: ### Where dialogs render
L21: The window's [Root](./root.md) automatically mounts and renders dialogs. Open the window with `gpui_kit::open_window`, or wrap the application view in `Root::new`. Application views do not render overlay layers themselves.
L23: ### Basic Dialog
L26: window.open_dialog(cx, |dialog, _, _| {
L27:     dialog
```

### 应用打包

来源：[website/docs/packaging.md](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/packaging.md)

```text
L3: description: Turn a GPUI Kit release build into a macOS app and DMG, a Windows installer, or a Linux tarball and DEB.
L9: `cargo build --release` produces an executable, not an installer. GPUI Kit supplies the application and UI layers; your application owns its package identity, icons, external files, installer, signing, and release tests. Start with [Installation](./installation.md) and [Getting Started](./getting-started.md), then build and package **on each target platform**. This guide uses the repository's `hello_world` binary to make paths concrete. Replace `hello_world`, `HelloWorld`, the sample version, and `com.example.helloworld` with your application's values before shipping.
L15: | macOS | A `.app` inside a ZIP or tarball | A `.app` distributed in a `.dmg` | Finder recognizes the app bundle; direct distribution needs Developer ID signing and notarization. A DMG is a delivery container, not an installer that writes files automatically. |
L16: | Windows | ZIP containing an `.exe` and its files | An Inno Setup `.exe` installer | The installer registers the app, shortcuts, and uninstall behavior. Renaming a ZIP to `.exe` does not provide that behavior. |
L17: | Linux | `.tar.gz` with the binary and resources | Distribution package such as `.deb` | A DEB declares package identity and dependencies, installs a desktop entry and icon, and supports package manager upgrade/removal. A tarball needs an explicit install process. |
L19: Choose only the formats your users need. A `.deb` does not cover RPM distributions, and a Windows ZIP is useful for portable use even when you also provide an installer. [Apple's distribution guide](https://developer.apple.com/documentation/xcode/packaging-mac-software-for-distribution), [Microsoft's packaging overview](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/packaging/), and the [Debian binary package guide](https://www.debian.org/doc/debian-policy/ap-pkg-binarypkg.html) describe the platform rules.
L29: The output is `target/release/hello_world` on macOS/Linux or `target\release\hello_world.exe` on Windows. In your own application, run `cargo build --release --locked` from its project directory and use its binary name. Keep the `Cargo.lock` used to build the release, and record the target OS, CPU architecture, app version, and build revision with the artifact. Build separately for each architecture you publish; a successful x86-64 build does not establish an Arm build.
L36: - Review native dynamic libraries and system services used by *your* feature set. A compile-time development package is not necessarily a runtime dependency. For example, a Linux GPUI window needs a working graphical session and Vulkan driver; a machine with only the Vulkan loader cannot render it. See [Installation](./installation.md).
L40: ## macOS
L42: A macOS application is a directory with a defined bundle layout: `Contents/MacOS` holds the executable, `Contents/Resources` holds resources, and `Contents/Info.plist` declares the executable and bundle identifier. [Apple documents the bundle structure](https://developer.apple.com/documentation/bundleresources/placing-content-in-a-bundle). On a Mac, assemble a minimal local test bundle from this repository's example:
L45: mkdir -p dist/HelloWorld.app/Contents/MacOS dist/HelloWorld.app/Contents/Resources
L46: cp target/release/hello_world dist/HelloWorld.app/Contents/MacOS/hello_world
L51:   <key>CFBundleName</key><string>HelloWorld</string>
L52:   <key>CFBundleDisplayName</key><string>HelloWorld</string>
L53:   <key>CFBundleIdentifier</key><string>com.example.helloworld</string>
L54:   <key>CFBundleExecutable</key><string>hello_world</string>
L55:   <key>CFBundlePackageType</key><string>APPL</string>
L56:   <key>CFBundleShortVersionString</key><string>{{gpui_kit_version}}</string>
L57:   <key>CFBundleVersion</key><string>1</string>
L64: That bundle has no custom icon. For your app, add an `.icns` in `Contents/Resources`, declare `CFBundleIconFile`, and copy external files under `Resources`; resolve them from the bundle location rather than the caller's working directory. Update both version fields for each release. The example above is a **local bundle test**, not a signed public release.
```

### 更新边界

来源：[website/docs/auto-update.md](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/website/docs/auto-update.md)

```text
L3: description: Plan safe update checks, verified downloads, installation, and restart for GPUI Kit desktop apps.
L9: An update has two separate jobs: the application presents a useful, responsive experience, and the release system delivers a trustworthy replacement. GPUI Kit gives you the UI, state, and task tools for the first job. **Your application and its distribution system own the update policy, release metadata, verification, installation, and recovery.** GPUI Kit does not select a release or replace an installed app. Start with [Packaging Desktop Apps](./packaging.md): an updater must match the package format and installation scope you ship.
L11: ## Choose who owns installation
L13: | Distribution | Recommended update owner | Reason |
L15: | A writable, single-binary portable installation | The app can use a binary updater such as `self_update`, with a controlled restart | There is one executable to replace and no package database to keep in sync. |
L16: | macOS `.app` delivered in a DMG | A bundle-aware updater or a new signed and notarized app/DMG | Updating only `Contents/MacOS/<binary>` changes the signed bundle. Replace and validate the complete `.app`. |
L17: | Windows Inno Setup `.exe` installer | The installer | It owns installed files, shortcuts, permissions, version identity, and uninstall records. A running process may lock its executable. |
L21: Do not silently switch an installation from one owner to another. In particular, a binary self-updater does not replace the Inno Setup installation flow, a macOS bundle update, or a Linux package upgrade. A signed update to a portable binary still needs a writable destination and a platform-specific restart strategy.
L27: 1. **Identity and policy:** stable application/package identity, current version, update channel (`stable` or `beta`, for example), minimum supported version, and whether downgrades are allowed. Compare versions with a defined version scheme; do not compare arbitrary version strings lexically.
L29: 3. **Release metadata:** version, human-readable notes, artifact URL, size if known, and a digest or signature. Serve metadata and assets over HTTPS and treat redirects and release-host credentials as part of your trust policy. Do not embed a private release token in the distributed executable.
L30: 4. **Verification:** check the downloaded bytes against the selected release's expected digest **before** installation, then apply the platform's signature or publisher-authenticity checks. A `SHA256SUMS` file hosted beside the artifact catches corruption and mismatches; by itself it does not prove who published either file. Use a trusted signing key, signed metadata, or the applicable platform signature when publisher authenticity matters.
L31: 5. **Atomicity and recovery policy:** stage the artifact outside the live installation, reserve space, verify the full payload, and define what happens if extraction, replacement, relaunch, or the new version's startup fails. Do not claim rollback unless your installer actually retains and restores a known-good version.
```

### CI配置

来源：[.github/workflows/ci.yml](https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/.github/workflows/ci.yml)

```text
L81:         run: cargo check -p gpui-component --no-default-features --locked
L93:           - target: aarch64-apple-darwin
L94:             run_on: macos-latest
L95:           - target: x86_64-linux-gnu
L96:             run_on: ubuntu-latest
L97:           - target: x86_64-windows-msvc
L98:             run_on: windows-latest
L132:       # this single run also covers the headless UI integration tests in
L136:           cargo test --workspace --exclude gpui-shell
L142:           cargo test --workspace --exclude gpui-shell
L145:       # `crates/kit/tests/rendering.rs` needs a real renderer and is opted
L151:       - name: UI rendering tests
L154:           cargo test --workspace --exclude gpui-shell
L156:           $TEST_EXCLUDES --test rendering
L163:         run_on: [ubuntu-latest, macos-latest, windows-latest]
L179:         run: cargo test -p gpui-kit --features gpui-fast,test-support --locked
L182:           cargo test -p gpui-component --features gpui-fast,test-support
L185:         run: cargo test -p gpui-webview --features gpui-fast,gpui-kit/gpui-fast --locked
L194:         run_on: [ubuntu-latest, macos-latest, windows-latest]
L224:         run: cargo check -p gpui-kit --features gpui-fast --target wasm32-unknown-unknown --locked
L242:           cargo test -p gpui-shell --release --lib -- \
L248:           cargo test -p gpui-shell --release tests::benchmark
L249:           cargo test -p gpui-shell --release --doc
L255:         run: cargo check -p gpui-shell --no-default-features --features quickjs
```

### 发布GPUI源码的原生API定位

来源： https://docs.rs/crate/gpui-pre/0.3.10/source/ （由0.3.10发布crate直接读取）。

**原生菜单/文件对话框/退出**

```text
src/app.rs:335: pub fn on_reopen<F>(&self, mut callback: F) -> &Self
src/app.rs:2015: pub fn prompt_for_paths(
src/app.rs:2028: pub fn prompt_for_new_path(
src/app.rs:2052: pub fn restart(&mut self) {
src/app.rs:2886: pub fn on_window_closed(
src/app.rs:2923: pub fn set_menus(&self, menus: impl IntoIterator<Item = Menu>) {
src/app/test_context.rs:249: pub fn on_quit(&mut self, f: impl FnOnce() + 'static) {
```

**窗口/HUD相关字段**

```text
src/platform.rs:1977: pub kind: AtlasTextureKind,
src/platform.rs:2586: pub titlebar: Option<TitlebarOptions>,
src/platform.rs:2589: pub focus: bool,
src/platform.rs:2592: pub show: bool,
src/platform.rs:2595: pub kind: WindowKind,
src/platform.rs:2600: pub is_movable: bool,
src/platform.rs:2621: pub is_resizable: bool,
src/platform.rs:2631: pub window_background: WindowBackgroundAppearance,
src/platform.rs:2634: pub app_id: Option<String>,
src/platform.rs:2668: pub titlebar: Option<TitlebarOptions>,
src/platform.rs:2672: pub kind: WindowKind,
src/platform.rs:2676: pub is_movable: bool,
src/platform.rs:2687: pub is_resizable: bool,
src/platform.rs:2697: pub focus: bool,
src/platform.rs:2700: pub show: bool,
src/platform.rs:2710: pub app_id: Option<String>,
src/profiler/journal.rs:137: pub kind: &'static str,
```

### 成品终端/托盘的限定扫描

扫描仅覆盖此固定tag的`crates/**/*.rs`，匹配portable_pty、alacritty_terminal、wezterm、vte::、struct/trait Terminal、PtySystem，以及tray_icon/TrayIcon/system tray/StatusItem。未命中只表示此次未找到，不构成普遍不支持证明。

- terminal/PTY: 无命中
- tray: 无命中

## 附录：完整上游许可证（访问于2026-10-29，固定tag）

### LICENSE-APACHE

原件：https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/LICENSE-APACHE

```text
Copyright 2024 - 2026 Longbridge <https://longbridge.com>

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

	http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.


                              Apache License
                        Version 2.0, January 2004
                     http://www.apache.org/licenses/

TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

1. Definitions.

   "License" shall mean the terms and conditions for use, reproduction,
   and distribution as defined by Sections 1 through 9 of this document.

   "Licensor" shall mean the copyright owner or entity authorized by
   the copyright owner that is granting the License.

   "Legal Entity" shall mean the union of the acting entity and all
   other entities that control, are controlled by, or are under common
   control with that entity. For the purposes of this definition,
   "control" means (i) the power, direct or indirect, to cause the
   direction or management of such entity, whether by contract or
   otherwise, or (ii) ownership of fifty percent (50%) or more of the
   outstanding shares, or (iii) beneficial ownership of such entity.

   "You" (or "Your") shall mean an individual or Legal Entity
   exercising permissions granted by this License.

   "Source" form shall mean the preferred form for making modifications,
   including but not limited to software source code, documentation
   source, and configuration files.

   "Object" form shall mean any form resulting from mechanical
   transformation or translation of a Source form, including but
   not limited to compiled object code, generated documentation,
   and conversions to other media types.

   "Work" shall mean the work of authorship, whether in Source or
   Object form, made available under the License, as indicated by a
   copyright notice that is included in or attached to the work
   (an example is provided in the Appendix below).

   "Derivative Works" shall mean any work, whether in Source or Object
   form, that is based on (or derived from) the Work and for which the
   editorial revisions, annotations, elaborations, or other modifications
   represent, as a whole, an original work of authorship. For the purposes
   of this License, Derivative Works shall not include works that remain
   separable from, or merely link (or bind by name) to the interfaces of,
   the Work and Derivative Works thereof.

   "Contribution" shall mean any work of authorship, including
   the original version of the Work and any modifications or additions
   to that Work or Derivative Works thereof, that is intentionally
   submitted to Licensor for inclusion in the Work by the copyright owner
   or by an individual or Legal Entity authorized to submit on behalf of
   the copyright owner. For the purposes of this definition, "submitted"
   means any form of electronic, verbal, or written communication sent
   to the Licensor or its representatives, including but not limited to
   communication on electronic mailing lists, source code control systems,
   and issue tracking systems that are managed by, or on behalf of, the
   Licensor for the purpose of discussing and improving the Work, but
   excluding communication that is conspicuously marked or otherwise
   designated in writing by the copyright owner as "Not a Contribution."

   "Contributor" shall mean Licensor and any individual or Legal Entity
   on behalf of whom a Contribution has been received by Licensor and
   subsequently incorporated within the Work.

2. Grant of Copyright License. Subject to the terms and conditions of
   this License, each Contributor hereby grants to You a perpetual,
   worldwide, non-exclusive, no-charge, royalty-free, irrevocable
   copyright license to reproduce, prepare Derivative Works of,
   publicly display, publicly perform, sublicense, and distribute the
   Work and such Derivative Works in Source or Object form.

3. Grant of Patent License. Subject to the terms and conditions of
   this License, each Contributor hereby grants to You a perpetual,
   worldwide, non-exclusive, no-charge, royalty-free, irrevocable
   (except as stated in this section) patent license to make, have made,
   use, offer to sell, sell, import, and otherwise transfer the Work,
   where such license applies only to those patent claims licensable
   by such Contributor that are necessarily infringed by their
   Contribution(s) alone or by combination of their Contribution(s)
   with the Work to which such Contribution(s) was submitted. If You
   institute patent litigation against any entity (including a
   cross-claim or counterclaim in a lawsuit) alleging that the Work
   or a Contribution incorporated within the Work constitutes direct
   or contributory patent infringement, then any patent licenses
   granted to You under this License for that Work shall terminate
   as of the date such litigation is filed.

4. Redistribution. You may reproduce and distribute copies of the
   Work or Derivative Works thereof in any medium, with or without
   modifications, and in Source or Object form, provided that You
   meet the following conditions:

   (a) You must give any other recipients of the Work or
       Derivative Works a copy of this License; and

   (b) You must cause any modified files to carry prominent notices
       stating that You changed the files; and

   (c) You must retain, in the Source form of any Derivative Works
       that You distribute, all copyright, patent, trademark, and
       attribution notices from the Source form of the Work,
       excluding those notices that do not pertain to any part of
       the Derivative Works; and

   (d) If the Work includes a "NOTICE" text file as part of its
       distribution, then any Derivative Works that You distribute must
       include a readable copy of the attribution notices contained
       within such NOTICE file, excluding those notices that do not
       pertain to any part of the Derivative Works, in at least one
       of the following places: within a NOTICE text file distributed
       as part of the Derivative Works; within the Source form or
       documentation, if provided along with the Derivative Works; or,
       within a display generated by the Derivative Works, if and
       wherever such third-party notices normally appear. The contents
       of the NOTICE file are for informational purposes only and
       do not modify the License. You may add Your own attribution
       notices within Derivative Works that You distribute, alongside
       or as an addendum to the NOTICE text from the Work, provided
       that such additional attribution notices cannot be construed
       as modifying the License.

   You may add Your own copyright statement to Your modifications and
   may provide additional or different license terms and conditions
   for use, reproduction, or distribution of Your modifications, or
   for any such Derivative Works as a whole, provided Your use,
   reproduction, and distribution of the Work otherwise complies with
   the conditions stated in this License.

5. Submission of Contributions. Unless You explicitly state otherwise,
   any Contribution intentionally submitted for inclusion in the Work
   by You to the Licensor shall be under the terms and conditions of
   this License, without any additional terms or conditions.
   Notwithstanding the above, nothing herein shall supersede or modify
   the terms of any separate license agreement you may have executed
   with Licensor regarding such Contributions.

6. Trademarks. This License does not grant permission to use the trade
   names, trademarks, service marks, or product names of the Licensor,
   except as required for reasonable and customary use in describing the
   origin of the Work and reproducing the content of the NOTICE file.

7. Disclaimer of Warranty. Unless required by applicable law or
   agreed to in writing, Licensor provides the Work (and each
   Contributor provides its Contributions) on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
   implied, including, without limitation, any warranties or conditions
   of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
   PARTICULAR PURPOSE. You are solely responsible for determining the
   appropriateness of using or redistributing the Work and assume any
   risks associated with Your exercise of permissions under this License.

8. Limitation of Liability. In no event and under no legal theory,
   whether in tort (including negligence), contract, or otherwise,
   unless required by applicable law (such as deliberate and grossly
   negligent acts) or agreed to in writing, shall any Contributor be
   liable to You for damages, including any direct, indirect, special,
   incidental, or consequential damages of any character arising as a
   result of this License or out of the use or inability to use the
   Work (including but not limited to damages for loss of goodwill,
   work stoppage, computer failure or malfunction, or any and all
   other commercial damages or losses), even if such Contributor
   has been advised of the possibility of such damages.

9. Accepting Warranty or Additional Liability. While redistributing
   the Work or Derivative Works thereof, You may choose to offer,
   and charge a fee for, acceptance of support, warranty, indemnity,
   or other liability obligations and/or rights consistent with this
   License. However, in accepting such obligations, You may act only
   on Your own behalf and on Your sole responsibility, not on behalf
   of any other Contributor, and only if You agree to indemnify,
   defend, and hold each Contributor harmless for any liability
   incurred by, or claims asserted against, such Contributor by reason
   of your accepting any such warranty or additional liability.

END OF TERMS AND CONDITIONS
```

### crates/assets/LICENSE-LUCIDE

原件：https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/crates/assets/LICENSE-LUCIDE

```text
ISC License

Copyright (c) 2026 Lucide Icons and Contributors

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

---

The following Lucide icons are derived from the Feather project:

airplay, alert-circle, alert-octagon, alert-triangle, aperture, arrow-down-circle, arrow-down-left, arrow-down-right, arrow-down, arrow-left-circle, arrow-left, arrow-right-circle, arrow-right, arrow-up-circle, arrow-up-left, arrow-up-right, arrow-up, at-sign, calendar, cast, check, chevron-down, chevron-left, chevron-right, chevron-up, chevrons-down, chevrons-left, chevrons-right, chevrons-up, circle, clipboard, clock, code, columns, command, compass, corner-down-left, corner-down-right, corner-left-down, corner-left-up, corner-right-down, corner-right-up, corner-up-left, corner-up-right, crosshair, database, divide-circle, divide-square, dollar-sign, download, external-link, feather, frown, hash, headphones, help-circle, info, italic, key, layout, life-buoy, link-2, link, loader, lock, log-in, log-out, maximize, meh, minimize, minimize-2, minus-circle, minus-square, minus, monitor, moon, more-horizontal, more-vertical, move, music, navigation-2, navigation, octagon, pause-circle, percent, plus-circle, plus-square, plus, power, radio, rss, search, server, share, shopping-bag, sidebar, smartphone, smile, square, table-2, tablet, target, terminal, trash-2, trash, triangle, tv, type, upload, x-circle, x-octagon, x-square, x, zoom-in, zoom-out

The MIT License (MIT) (for the icons listed above)

Copyright (c) 2013-present Cole Bemis

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

### LICENSE-DOCS.md

原件：https://github.com/longbridge/gpui-kit/blob/99576db58ec8103417c41a092e891b4022d24e7e/LICENSE-DOCS.md

```text
# Documentation license

From the revision that adds this notice, documentation prose and original illustrations for which GPUI Kit holds the rights needed to grant this license in `website/docs/`, `website/base/`, `website/component/`, `website/shell/`, and the corresponding `website/zh-CN/` directories are also offered under the [Creative Commons Attribution 4.0 International license (CC BY 4.0)](https://creativecommons.org/licenses/by/4.0/).

When copying or adapting that material, credit **GPUI Kit**, link to the source page and the CC BY 4.0 license, and indicate if you made changes. For example: “Adapted from GPUI Kit, [page title] ([source page URL]), [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).” The license applies to copyrightable material; it does not require attribution for learning an idea or using a fact without copying protected expression.

This documentation license does **not** apply to software source, website implementation, or code examples and snippets within documentation. Those remain under the repository's [Apache-2.0 license](LICENSE-APACHE). Third-party contributions and material, including images, fonts, icons, and quotations, are not relicensed here without the relevant rights holder's permission; their existing licenses and notices continue to apply. Existing Apache-2.0 permissions are not withdrawn. This additional CC BY 4.0 offer does not retroactively change the terms of earlier repository revisions or previously published snapshots.
```

