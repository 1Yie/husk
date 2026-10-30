# Husk GPUI

Husk 的原生桌面入口，使用 GPUI-Kit 和现有 `husk-host`。当前保留独立
`husk-gpui` 二进制，不改变默认桌面发行入口。

## 运行

```sh
nice -n 10 cargo build --locked -p husk-gpui --bin husk-gpui
./target/debug/husk-gpui
```

默认启用 semantic 功能。只验证界面、宿主和终端时，可以使用：

```sh
nice -n 10 cargo build --locked -p husk-gpui --no-default-features --bin husk-gpui
./target/debug/husk-gpui
```

只对构建使用 `nice`，界面应以正常优先级运行。追加 `--settings` 可直接打开设置页。

配置及数据沿用应用目录，工作区数据格式不变。验证时请使用下面的隔离工具，
不要用生产会话做自动化输入。

Linux 需要可工作的 Vulkan 驱动，以及 Fontconfig、FreeType、Wayland、
XKB/XKB-X11 的开发依赖。Fedora 的运行库不等同于链接所需开发包：

```sh
sudo dnf install fontconfig-devel freetype-devel wayland-devel \
  libxkbcommon-devel libxkbcommon-x11-devel vulkan-loader-devel
```

macOS 原生入口以 **15.0+** 为目标；Windows 使用 MSVC 工具链。
跨平台编译检查不能代替各平台实机验收。安全限定的工作区改动读取已支持
Linux/Android/macOS；macOS 后端目前只通过交叉编译，尚未实机验收。Windows
的改动面板会明确显示不支持，不能视为功能对等。

## 隔离的端到端验收

```sh
nice -n 10 cargo build --locked -p husk-gpui --no-default-features \
  --bin husk-gpui --example seed_reference
python3 crates/app-gpui/scripts/native-smoke.py
```

工具会：

1. 创建独立的 HOME、配置、数据和临时 Git 工作区。
2. 启动仅监听 loopback 的模型服务，不读取真实账号，不访问外部模型。
3. 通过公开宿主接口完成流式会话，再启动真实窗口检查历史恢复。
4. 保留临时目录及窗口日志，关闭窗口后停止本地服务。

窗口中的内容明确是验收数据；生产程序没有演示消息注入逻辑。
在 KDE 桌面上可追加 `--screenshot`，脚本会将隔离窗口截图保存到临时目录的
`native.png`；窗口仍需关闭后脚本才会退出。截图前后会核对活动窗口的进程，但无法
消除焦点切换竞态；需要严格隔离时应在独立桌面会话中运行。

## 已完成

以下为已落地的实现，不代表跨平台实机、打包和性能验收全部完成。

- [x] `app`：启动、主窗口组合、导航及退出回收。
- [x] `bridge`：后台宿主、工作区/会话路由、审批与取消快速通道。
- [x] `features/chat`：纯消息投影、原生 Markdown、变高虚拟列表和输入。
- [x] `features/terminal`：终端模拟、原生绘制、输入、标签及分屏。
- [x] `features/workspace`：后台 Git 读取、请求代际和只读 diff。
- [x] `ui`：应用资源、Keyline 图标、设计 token 和布局尺寸。
- [x] `platform`：路径及系统接口。
- [x] `husk-terminal`：共享的 owner-scoped PTY 服务与进程回收。

主线程不执行 Git、PTY 启动或宿主初始化。会话身份始终包含工作区 root 和
session id。隐藏终端不终止进程；关闭标签释放其 PTY；真正退出前等待 reaper。
终端标签及分屏显示程序设置的标题；标题会限长并过滤控制字符和双向文本控制符。
终端分隔条可拖动调整；分屏尺寸在切换标签或隐藏终端后保留。终端区域支持新建、
关闭当前窗格、切换标签和分屏快捷键，按钮提示会显示当前绑定；具体组合见
[`bind_keys`](src/features/terminal/deck.rs)。关闭最后一个窗格后仍可通过快捷键新建。
快速关闭再新建时，启动任务在后台限时等待回收中的 PTY 释放空位；取消或退出会停止等待，
不会扩大共享服务的终端数量上限。共享服务的即时 `open` 接口在容量不足时直接返回错误。
仅在当前模型显式配置 Fast 能力时显示快速模式开关；该开关按会话保存。
设置页支持会话记忆和自动提取记忆的全局开关；保存后通知已打开会话，当前回合可能仍沿用
原设置。自动提取会使用模型调用，仅在启用会话记忆时生效。读取偏好失败时禁用修改，
不会用默认值覆盖损坏的数据；设置页请求提供待处理状态，其错误留在设置页而非聊天输入区。
会话列表支持右键重命名，并可分别固定到项目或固定到全局会话列表。
当前工作区的会话可在二次确认后删除；其他工作区需先切换过去，避免隐式导航。
运行中发送的后续消息会进入会话队列并写入会话存储；输入框上方可以查看、编辑正文、
上移、下移和移除。编辑使用独立草稿，不改动普通输入框中的文字和附件；排队消息已有的
附件块保持不变，不重新读取源文件。
重启后恢复的队列不会自动执行，可使用“继续”启动；队列在操作时发生变化会先刷新，
不会用旧列表覆盖新消息。保存失败或队列变化时保留编辑草稿，可复制正文后取消。
聊天输入框支持通过原生文件选择器添加多个文件或图片、移除附件，以及仅发送附件。
工作区外的附件会暂存到工作区的 `.husk/attachments/`，以便会话沙箱访问；
文本附件只内联前 32 KiB，较大的文件会标记为截断。附件读取不阻塞界面，
提交失败时会恢复附件和文本，并保留此后输入的新草稿。
Linux/macOS 的附件加载使用非阻塞、逐级不跟随符号链接的文件打开；暂存使用
目录描述符相对操作。这不是针对目录重命名或挂载边界的完整安全沙箱；Windows
仍使用普通路径操作，尚未完成原生安全验收。

## 参考布局

默认布局尺寸集中在 [`ui/layout.rs`](src/ui/layout.rs)，单位为逻辑像素；
分栏可以调整，窄窗口会隐藏改动面板。字体沿用宿主外观偏好，解析和回退策略见
[`ui/theme.rs`](src/ui/theme.rs)。图标来自 Tauri 使用的 Keyline 原始 SVG。

资源版权及字体许可在 `assets/licenses`。这些资源不自动改用应用的 MIT 许可；
生成正式分发包前需核对完整的 MiSans 分发条款并保留资源许可。

## 验证

```sh
./scripts/check-deps.sh
cargo fmt -p husk-gpui -p husk-terminal --check
nice -n 10 cargo test --locked -p husk-gpui -p husk-terminal \
  --no-default-features --lib
# 额外启用无头 GPUI 窗口和文件选择器交互测试
nice -n 10 cargo test --locked -p husk-gpui \
  --no-default-features --features test-support --lib
```

本地纯逻辑/宿主测试、真实 PTY 测试、原生窗口测试分开报告。中文文本渲染验收，
不等同于输入法候选窗、组合输入和所有显示服务器行为均已验证。

## 未完成与待验收

- [ ] 独立电脑控制 HUD、可见提示和停止入口；完成前宿主明确拒绝电脑控制授权，
  不能通过提高权限模式绕过这一限制。
- [ ] 系统托盘。
- [ ] 通知点击导航、通知声音开关。
- [ ] 更新安装。
- [ ] 全部设置页，以及自定义配色、字体等其他外观设置；浅色、深色和跟随系统选择已沿用宿主偏好。
- [ ] 附件拖放、剪贴板粘贴、图片缩略图及完整预览能力。
- [ ] Windows 安全限定的工作区改动读取；macOS 读取后端仍待实机验收。
- [ ] Windows 通知打包与实机验收。
- [ ] Windows/macOS 整体实机验收、安装包及性能基准。

系统通知遵循宿主的通知开关，仅发送固定提示文案，不包含会话标题、提示词或工具内容；
偏好读取失败时关闭投递。macOS 需要应用包和系统授权，Windows 需要安装包注册的
AUMID，当前尚未完成 Windows 通知打包与实机验收。通知声音开关和点击跳转尚未接入。

默认发布入口保持不变；上述清单完成前，原生入口不能视为完整替换。

## 工作区读取边界

工作区扫描只读取所选工作区内的 `.gitignore`，不加载祖先目录或全局排除规则；
因此结果可能包含 Git 全局忽略的文件。单文件超过 128 KiB 时不展示补丁，扫描总预算
耗尽时明确报错。Git 元数据、硬链接和挂载边界不属于此读取器的安全沙箱。
GPUI 使用 scoped 接口；旧 `worktree_changes` 在 Unix 上已限制后续文件预览，
但其 gix status 阶段仍可能读取特殊 ignore/属性文件、
发生阻塞或运行配置的过滤器，不应被当作不可信工作区的安全扫描器。

