# WebKitGTK 透明窗口前置调研

- 日期：**2026-11-01 CST**，由本机 `date` 确认。
- 性质：dated research record。只记录今天能确证的事实与待验证假设，不声明 Linux 材质已可行。
- 工程基线：`main`，`16acc6a`（Windows 外框已合入，Windows 材质在 `task/windows-backdrop`）。
- `Question:` 在本仓库当前的 WebKitGTK/KWin 栈上，能不能让主窗口正确绘制为透明窗口，使合成器侧的模糊可见？
- `Conclusion:` **未定**。需要下文 E1–E3 在 KDE 机器上的结果才能下结论。

## 为什么这是前置问题

KDE 的窗口模糊属于合成器侧行为：X11 是给窗口设 `_KDE_NET_WM_BLUR_BEHIND_REGION`，Wayland 是把 `org_kde_kwin_blur_manager` 绑定在窗口**自己**的 surface 连接上。两者都只模糊窗口背后的像素，窗口本身不透明时没有任何可见效果。也就是说 Linux 的模糊代码只有几十行，真正未知的是透明窗口能否正确绘制；不解决它，写出来的模糊代码会永远返回 `false`。

## 已确证的事实

1. **主窗口在 Linux 被配置成不透明。** `crates/app-tauri/tauri.linux.conf.json` 把窗口写成 `"transparent": false`、`"backgroundColor": "#fafafa"`，覆盖基础配置 `crates/app-tauri/tauri.conf.json:23-24` 的 `"transparent": true`、`"backgroundColor": "#00000000"`。
2. **原因有复现记录。** `crates/app-tauri/backend/main.rs:197-206`：表面起来即 alpha=0，WebKit 内容不落到窗口里，合成器透出桌面或陈旧缓冲，只有鼠标扫过被 WebKit 损伤的矩形短暂留下；在 x11 与 Wayland、软件 GL、`WEBKIT_DISABLE_COMPOSITING_MODE=1` 下都复现；`transparent: false` 每次整窗正常。
3. **透明度只能在建窗时决定。** tao 仅在建窗时按 `transparent` 设 RGBA visual 与 `app_paintable`（`tao-0.35.3/src/platform_impl/linux/window.rs:131-143`），运行中无法切换。所以 Linux 上不存在「先透明、失败再回实色」的运行时回退，只能在建窗前判定。
4. **「忘了设透明背景」不是原因。** wry 在 `transparent` 时把 WebKitWebView 的背景设为 `RGBA(0,0,0,0)`（`wry-0.55.1/src/webkitgtk/mod.rs:289`），并且整窗透明在创建时就已传入。
5. **版本基线。** 应用固定 `webkit2gtk = "=2.0.2"`（`v2_38`）与 `gtk = "0.18"`（`crates/app-tauri/Cargo.toml:67,70`）；历史记录中的实际运行版本是 WebKitGTK 2.52.1、KWin 6.7、NVIDIA 驱动 615（`crates/app-tauri/backend/main.rs:175`）。
6. **实验前必须固定渲染变量。** 启动时会按 GPU 与会话写入环境（`pin_linux_rendering_env`，`crates/app-tauri/backend/main.rs:231` 起），并可被 `~/.config/husk/settings.toml` 的 `renderer`/`cpu_raster`/`gpu_acceleration` 覆盖。不固定这些值，两次运行的结论不可比。

## 待验证假设

每条都写成可证伪的形式，并给出区分它的实验与判据。

- **H1 GTK 窗口自身背景盖住了内容。** `app_paintable` 为真时由应用负责绘制，若没有显式背景图案，GTK 可能按主题色清除窗口区域。
  判别：显式把窗口背景图案置空（`gdk_window_set_background_pattern(window, None)` 后 `queue_draw`）。
  若内容因此出现 → 缺陷在 GTK 侧绘制，与 WebKit 无关。
- **H2 WebKit 的加速合成路径无法合成进 RGBA 表面。**（dmabuf/GL 路径）
  判别：保持 compositing 开启、只加 `WEBKIT_DISABLE_DMABUF_RENDERER=1`；再对比历史上已试过的 `WEBKIT_DISABLE_COMPOSITING_MODE=1`。
  若仅在禁用 dmabuf 后内容出现 → 原因是 dmabuf 渲染器，可用环境变量或版本升级规避。
- **H3 只发生在特定 GPU 栈（NVIDIA egl-wayland）。** 仓库已记录该栈的多个已知问题（`__NV_DISABLE_EXPLICIT_SYNC`、dmabuf SHM 等，`backend/main.rs:175-195`）。
  判别：`LIBGL_ALWAYS_SOFTWARE=1` 配合 `WEBKIT_SKIA_ENABLE_CPU_RENDERING=1`，或在非 NVIDIA 机器上重跑 E2/E3。
  若只在 NVIDIA 上失败 → 可行性与设备绑定，门控需要读 GPU。
- **H4 KWin 或合成器不接受这个窗口的 alpha。** 判别：E1 的纯 GTK3 半透明窗口（无 WebView）。
  若纯 GTK 也坏 → 与 WebKit 无关，要换表面类型或窗口设置；若正常 → 排除此项。
- **H5 窗口 flags 组合有问题。** 无边框 + RGBA visual + `titleBarStyle: Overlay` 的组合可能让合成器走另一条路径。
  判别：E3 换成带 decorations 的同一窗口再试一次。
  若带 decorations 正常 → 是 flag 组合问题，而 macOS 式外框在 Linux 上需要考虑是否保留装饰。

## 实验顺序

每步记录：`echo $XDG_SESSION_TYPE`、`GDK_BACKEND`、GPU 与驱动版本、`pkg-config --modversion webkit2gtk-4.1`、KWin 版本、`settings.toml` 三项取值、所用环境变量、以及截图。

- **E1 纯 GTK3 半透明窗口（最小隔离）。** 无 WebView：RGBA visual 窗口里画一个 50% 半透明矩形。
  判据：矩形后面能否看见桌面。这决定 H4，也决定「这个桌面能不能做透明窗口」。
- **E2 E1 加一个 WebKitWebView 子窗口。** webview 背景设为 `RGBA(0,0,0,0)`（与 wry 一致），HTML 里放一个不透明方块。
  判据：方块是否正常显示、窗口其余部分是否透明。这一条是「WebKitGTK 能否画在透明表面上」的直接答案。
- **E3 Husky 自身。** 临时去掉 `tauri.linux.conf.json` 里的 `transparent: false`（用一次性配置覆盖，不提交），`cargo tauri dev` 从 `crates/app-tauri` 启动；先 X11 再 Wayland。
  判据：整屏是否正常渲染；是否只有悬停区域出现；是否有闪烁或陈旧缓冲。
- **E4 仅在 E2 或 E3 失败时进行。** 按 H1 → H2 → H3 → H5 顺序，每次只改一个变量，记录组合与结果，直到内容出现或全部组合穷尽。

## 结果会喂给哪个决定

- E1 失败 → 该 KDE 会话不具备透明窗口能力，Linux 材质不可行；blur 提案不启动（或按环境排除）。
- E1、E2 都通过 → WebKitGTK 可用，Linux 提案可以直接写：建窗前判定能力，然后 X11 属性 / Wayland 协议。
- E1 通过、E2 在某个 H2/H3 组合下通过 → 可行但需要按 GPU 与渲染器判定；门控必须读取与 `pin_linux_rendering_env` 相同的输入，避免「应用自己把渲染器切成会失败的那一种」。
- 只有 H5 组合通过 → 需要放开窗口装饰或调整 flags，与 macOS 的外框形态冲突，属于设计问题而不是实现问题。
- 全部失败 → Linux 保持实色，并在提案里写明原因与已排除项。

## 未覆盖与未知

- 没有检索 WebKitGTK 与 wry 的上游 issue 跟踪器；也未实测 2.46 / 2.48 / 2.52 的版本矩阵。
- 当时复现记录没有写下 GPU 型号、KWin 版本与完整环境变量，故无法判断 H3 是否已经包含在原复现里。
- 没有验证 `transparent: true` 与 `decorations: false`、`titleBarStyle` 在 Linux 上的实际组合。
- KWin 模糊区域的实际半径、着色与失焦行为未在本轮验证（属于 blur 阶段）。
- Wayland 下的协议绑定需要新增直接依赖（`wayland-client`、`wayland-protocols-plasma`、`gdkwayland-sys`，X11 侧另有 `gdkx11`），目前都只是 tao/wry 带进来的传递依赖；是否新增尚未决定。

## 来源

- 本仓库：`crates/app-tauri/tauri.conf.json`、`crates/app-tauri/tauri.linux.conf.json`、`crates/app-tauri/backend/main.rs:175-206,231`、`crates/app-tauri/Cargo.toml:67,70`。
- 本地源码：tao **0.35.3** `src/platform_impl/linux/window.rs:131-143`；wry **0.55.1** `src/webkitgtk/mod.rs:289`。
- 兄弟记录 [`cross-platform-window-materials`](./cross-platform-window-materials.md)：KDE 两种模糊接口与「窗口必须透明」的平台事实，以及 `window-vibrancy` 的取舍。
