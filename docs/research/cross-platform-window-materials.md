# 跨平台窗口材质调研

- 日期：**2026-10-31 CST**，由本机 `date` 确认。
- 性质：dated research record，供方案讨论；候选方案尚未批准，本文不声明跨平台支持已实现。
- 工程基线：`main`，`28a8f1827a44fe8bbf156344f91f74682b71d6d9`；调研开始时已有 39 项未提交变更，保持原样。
- 验证环境：仅 macOS。读取了官方文档、Apple SDK 头文件、协议 XML 和上游源码；没有启动真实应用、编译 Rust，或运行 Windows、KDE、macOS 经典毛玻璃效果。
- 范围：让现有玻璃外框考虑 Windows 原生窗后模糊、KDE 支持，以及旧 macOS 的普通毛玻璃。保留单 WebView、实色内层卡片与透明外框作为待验证的最小候选。

## 结论与验证等级

`PASS` 表示指定 API、版本条件或协议机制已有一手依据；`FAIL` 表示某项等价关系或现成支持主张被源码否定；`UNVERIFIED` 表示实际集成、渲染或性能尚未验证。文档和源码通过，不等于对应平台运行验收通过。

| 项目 | 结论 | 本轮等级 |
| --- | --- | --- |
| A macOS | `NSGlassEffectView` 从 macOS 26 开始；旧 macOS 可用 `NSVisualEffectView` 的 `BehindWindow`，13.4 位于其 API 支持范围内 | API/版本 `PASS`；Husk 经典毛玻璃运行 `UNVERIFIED` |
| B Windows | Background/Desktop Acrylic 可采样窗后其他窗口；Mica 主要依据壁纸且为 opaque material，两者不能承诺同效果 | 材质区别、官方 DWM 下限 `PASS`；实际移动、缩放和系统策略响应 `UNVERIFIED` |
| C KDE | X11 blur hint 和旧 Wayland blur 协议都有真实实现；较新 KWin 使用另一代 background-effect 协议 | 协议机制 `PASS`；GTK/Tauri 同连接集成及目标发行版运行 `UNVERIFIED` |
| D Tauri/WebView | 单 WebView 可以继续作为候选；Tauri 2.11.5 没有现成 Linux `set_effects` apply 路径，调用成功不能充当激活证据 | “现成 Linux blur”主张 `FAIL`；跨平台端到端效果 `UNVERIFIED` |

## A. macOS：Liquid Glass 与普通毛玻璃

### 已核实事实

1. Apple 的 `NSGlassEffectView` 文档 availability 为 **macOS 26.0**；`NSVisualEffectView` 为 **macOS 10.10**。因此最低 macOS 13.4 可以覆盖经典 visual effect API，但不能使用 Liquid Glass 类作为唯一实现。[A1][A2]
2. Apple 明确定义 **Behind-window blending** 为使用窗口后方内容，和采样同一窗口内部内容的 **Within-window blending** 分开。普通窗后毛玻璃应使用 `BehindWindow`；只指定 material 或 CSS `backdrop-filter` 不能表达这一原生合成语义。[A2]
3. material 应按用途选择。Apple 指出并非所有 material 都支持透明，而且外观会随系统设置变化；`sidebar` 是其针对侧栏的示例。不能把枚举名当作固定颜色、模糊半径或跨系统外观承诺。[A2]
4. `window-vibrancy` **0.6.0** 的 `src/macos/internal.rs::apply_vibrancy` 创建 `NSVisualEffectView`，设置 material、`NSVisualEffectBlendingMode::BehindWindow` 和 state，再放入宿主 view；默认 state 是 `FollowsWindowActiveState`。该源码展示的是已有 AppKit 原生 view 路径，不要求增加 WebView。[A3]
5. SDK 的 `NSVisualEffectView.h` 将 state 分为 `FollowsWindowActiveState`、`Active`、`Inactive`。失焦行为属于可配置的材质状态，不能一律称作失效。appearance 也是 AppKit 外观体系的一部分；透明材质外观应与窗口的实际 appearance、亮暗主题一起验证。[A2][A4]
6. Apple 的 `accessibilityDisplayShouldReduceTransparency` 明确要求：值为 true 时避免半透明背景，例如使用 opaque windows；可通过 `accessibilityDisplayOptionsDidChangeNotification` 跟踪更改。强制 `Active` 不能成为绕过 Reduce Transparency 的策略。[A5]

### 最小候选建议

- macOS 26 及以上：保留已有 Liquid Glass 分支，按可用性和宿主创建结果判断。
- macOS 13.4–25：在现有 native owner 内选择普通 `NSVisualEffectView`，采用 `BehindWindow`；material、state、appearance 的具体选择仍需评审和真机比较。
- Reduce Transparency：实色外框；失焦按系统或明确选定的 state 变化，保留可读性。

这只是候选顺序。经典材质与 Liquid Glass 的视觉差异、遮罩与圆角、WebView 透明穿透、失焦和辅助功能切换均未运行验证。

## B. Windows：Acrylic、Mica 与 Blur 分开判断

### 材质与版本事实

- **Background Acrylic**：微软描述为揭示 desktop wallpaper 和当前应用后方的其他窗口；in-app acrylic 则采样应用内部内容。用户要求的“窗后内容模糊”对应前者，而不是任意名称带 Acrylic 的层。[W1]
- **Mica / Mica Alt**：微软称其为 opaque、dynamic material，结合主题和壁纸；为性能只采样壁纸一次，失焦时可退到 neutral color。它适合长驻窗口基础层，但不能作为“实时模糊后方其他应用”的等价替代。[W2]
- **DWM system backdrop**：`DWM_SYSTEMBACKDROP_TYPE` 的官方最低 client 是 **Windows 11 Build 22621（22H2）**；`DWMSBT_MAINWINDOW` 对应 Mica，`DWMSBT_TRANSIENTWINDOW` 对应 Desktop Acrylic，`DWMSBT_TABBEDWINDOW` 对应 Mica Alt。应区分正式 DWM API 下限和库里的 build 判断。[W3][W4]
- **旧 DWM Blur**：微软明确写出从 Windows 8 起 `DwmEnableBlurBehindWindow` 不再产生原先的 blur effect。不能由函数名或调用成功推导现代 Windows 已有毛玻璃。[W5]
- **Windows 10 与库路径**：`window-vibrancy` 0.6.0 的 Acrylic 在 build **17763（Windows 10 v1809）**及以上使用 `SetWindowCompositionAttribute` 的 Accent 路径；build **22523**及以上改走 `DWMWA_SYSTEMBACKDROP_TYPE`。其 Mica 在 build **22000**起使用未文档化的 `DWMWA_MICA_EFFECT`，在 22523 起也改走 system backdrop。库的 22523 判断早于微软正式文档的 22621 下限，应单独记录。Windows 10 Acrylic 可以列为候选，但须注明实际分支与 build，不能标成已验收的统一效果。[W6]

### 透明窗口、无原生标题栏与性能

微软 Win32 指南单列 WebView2 场景；Mica 指南还要求想露出材质的中间层保持透明，允许把内容扩展到 non-client area 并使用 transparent custom title bar。这为自绘标题栏、WebView 内容与原生 backdrop 共存提供依据；Husk 的实际窗口 styles、透明 WebView 背景和合成链仍需核实，不能由“无系统标题栏”单独判断支持或失败。[W2][W7]

`window-vibrancy` 0.6.0 README 的具体警告为：Acrylic 在 **Windows 10 v1903+、Windows 11 build 22000** 拖动/缩放性能差；Blur 在 **Windows 11 build 22621+** 拖动/缩放性能差。这些是上游已知风险，不是本轮测得的性能数值。此外，该版本 Windows 实现没有检查上述 `DwmSetWindowAttribute` 调用的 HRESULT；返回 `Ok` 也不足以证明 DWM 已应用材质。[W6][W8]

微软 Acrylic 指南指出 GPU 消耗，并列出 High Contrast、关闭 Transparency effects、Battery Saver 等实色条件；Background Acrylic 还受失焦等条件影响。Mica 指南也列出辅助功能、节能与设备条件下的 fallback。远程会话需要作为单独的部署条件验证；这些系统材质/控制器的行为，不能直接推广为低层 Accent 路径已实现同样的自动处理。[W1][W2]

### 最小候选建议

- Windows 11 22H2 及以上：若优先“看见窗后其他窗口的模糊”，先评估 Desktop Acrylic；若优先性能和 Windows 原生基础层外观，可让用户选择 Mica，但先确认其视觉语义。
- Windows 11 早期版本、Windows 10：分别评估 `window-vibrancy` 0.6.0 的实际分支和拖动/缩放代价。不要把库允许调用的版本统一写成官方 DWM 支持版本。
- 高对比、关闭透明、节能或效果不适用时，使用主题实色。远程环境、失焦与系统设置实时变化进入运行验收。

微软建议 Acrylic 用于 transient UI，并明确不推荐大面积 desktop acrylic 背景。[W1] Husk 仅用于外框可减少覆盖面积，但它仍是长驻窗口；需要对视觉目标、官方建议和性能作显式取舍。

## C. KDE：X11 与 Wayland，以及协议代际

### X11

`KWindowEffects` 的 XCB 实现先检查 compositing active，再从 root window 属性中检查 `_KDE_NET_WM_BLUR_BEHIND_REGION` 支持；`enableBlurBehind` 在目标窗口设置该 property，内容为矩形区域，禁用时删除 property。区域坐标在其 Qt 实现中按 device pixel ratio 转换。[K1]

KWin Plasma/6.4 的 BlurEffect 读取该 hint：有效矩形限定区域；存在而为空的 property 可表达整窗范围。hint 是向 compositor 提出请求；是否真正绘制还取决于效果加载、渲染能力和当前效果状态。[K2]

因此，X11 不能只检查 `XDG_CURRENT_DESKTOP=KDE`。需要实际 X11 window、合成状态、root 支持声明和 blur hint 的设置；关闭合成或效果时恢复主题实色。缩放变化也要重新核对区域坐标。

### 旧 Wayland 协议

`plasma-wayland-protocols/src/protocols/blur.xml` 定义：[K3]

- global/interface **`org_kde_kwin_blur_manager` version 1**；
- `create(new_id org_kde_kwin_blur, object wl_surface)` 和 `unset(wl_surface)`；
- `org_kde_kwin_blur` 的 `set_region(wl_region)`（允许 null）、`commit`、`release`（destructor）。

这不是 X11 property 的另一种传输方式，而是一套带 surface、region 和对象生命周期的 Wayland 协议。KWin Plasma/6.4 在 BlurEffect 创建时创建该 manager；析构时通过延迟 1000ms 的 timer 移除 global，以容纳合成器重启。因此 registry global 的存在是能力线索，也有短暂状态窗口；应处理 global removal 和 surface 重建。[K2]

`kwindowsystem` 的 Wayland 实现，在新 background-effect manager 未激活时，将旧 blur manager 的 `isActive()` 用于 `isEffectAvailable(BlurBehind)`。旧路径在 surface 可用后创建 blur 对象、设置 region 并 commit；surface 销毁时清理并等待新 surface。它借助 Qt Wayland 扩展、`QWindow` 和 native interface 获得原生 surface。[K4]

### 新协议与能力广告

读取的 KWin **master** 与 Plasma/6.4 分支不同：master 的 BlurEffect 调用 `backgroundEffectManager()->addBlurCapability()` / `removeBlurCapability()`；对应实现位于 `src/wayland/backgroundeffect_v1.cpp`。新 manager 的 global 名为 `ext_background_effect_manager_v1`、version 1，绑定时发送 capability，支持变化时重新发送；移除 blur capability 的通知经 100ms timer 合并。global 存在本身不能证明 blur capability 已启用。旧 `org_kde_kwin_blur_manager` 和新协议必须分别判断，不能将名字、对象或行为混同。[K2][K5][K6]

读取的 KWindowSystem master 同时生成两代客户端协议代码；`isEffectAvailable` 优先检查新 manager 及 `supportsBlur`，只有新 manager 未激活时才检查旧 manager。这个 Qt 实现可帮助理解协议选择，本轮没有决定为 Husk 复制其两代兼容路径。[K4]

本轮没有确认目标 KDE 发行版、下游 backport 或用户安装版本；不能把 master 的代码当成所有 Plasma 6 的发布契约。旧协议的 XML 已直接读取；新协议的完整客户端 XML/API 集成仍待针对目标发行版确认。

### GTK/Tauri 集成边界

`KWindowEffects` 是 Qt 封装，Husk 主窗口是 GTK/WebKitGTK，不能直接把 `QWindow` 调用搬过来。GTK3 上游头文件声明 `gdk_wayland_display_get_wl_display` 和 `gdk_wayland_window_get_wl_surface`，说明底层入口存在；可行性仍取决于 Rust binding、窗口 realize 生命周期和正确的后端分支。[G1][G2]

Wayland 请求里的 `wl_surface` 必须来自承载该窗口的同一客户端连接；另开一条 `wl_display` 连接不能拿 GTK surface 直接发请求。应使用 GTK 的 display/surface，在相同连接上发现 global、绑定协议对象、管理 region 与生命周期。**这条 GTK 同连接集成本轮为 UNVERIFIED**，不承诺现有依赖已足够或无需新增 binding。[K3][K4][G1][G2]

关闭 Blur 效果时，旧协议关注 registry global 的移除，新协议关注 capability 的变化；X11 关注 compositing 与支持 hint。其他桌面应按实际广告判断，有兼容协议也需验证，没有协议则使用实色。KWin 自身的 `supported()` 检查 OpenGL compositing，`isActive()` 还依赖有效资源与非锁屏状态；环境变量和 registry 的一次快照都不足以证明实时像素效果。[K2][K5]

## D. 当前工程约束与最小改动方向

以下是本轮协调者对本地源码的核实结果，供与外部证据对照；本研究作者没有重复整仓库审计。

- `crates/app-tauri/backend/ipc/sidebar_glass/macos.rs:137` 先检查 `available!(macos = 26)` 再检查 `NSGlassEffectView`，旧 macOS 返回 false；`crates/app-tauri/tauri.macos.conf.json` 的 `minimumSystemVersion` 为 13.4。候选经典毛玻璃用于补足这一版本区间。
- Tauri **2.11.5** 的 `src/window/mod.rs::set_effects`（约 2076 行）将工作交给 `run_on_main_thread`，闭包丢弃 `set_window_effects` 的结果；`src/vibrancy/mod.rs` 没有 Linux apply 分支，`None` 的 clear 路径只有 Windows。`set_effects` 返回 `Ok` 不能作为原生效果激活证据。[T1]
- 当前 IPC 已有 main 限定、输入验证、revision 队列与 native owner。普通 macOS 毛玻璃可优先在同 owner 内评估，保留现有校验与调度语义。
- Linux 直接依赖是 `gtk` 0.18、`webkit2gtk` 2.0.2、`gtk-layer-shell-sys` 0.7；layer-shell 用于 HUD。主窗口需要的是 compositor blur 接口，不能把 Main 改成 layer-shell 角色来替代。
- 通用 8px/14px 卡片有多处 `isMac` 门控，应与 traffic lights、native controls 等真实平台专属行为分别评估。布局推广属于后续实施范围。

**最小建议**：保留单 WebView；内层工作卡片继续 opaque，外框提供可穿透原生材质的透明区域。macOS 在现有 owner 内分支；Windows 先在已有 Tauri/window-vibrancy 能力内选材质；Linux 单独评估 GTK 原生句柄与 compositor 协议。这些事实没有要求增加第二棵 WebView，也不足以承诺所有窗口样式和平台都能原样复用。

## 建议支持矩阵

| 系统/后端 | 候选机制 | 关键条件 | 本轮验证层级 |
| --- | --- | --- | --- |
| macOS 26+ | 现有 Liquid Glass | 类可用、宿主创建成功、辅助功能允许 | Apple availability、当前代码核实；运行未验证 |
| macOS 13.4–25 | `NSVisualEffectView` / `BehindWindow` | material/state/appearance 合适；Reduce Transparency 时实色 | Apple 文档/SDK、库 0.6.0 源码；Husk 未实现验收 |
| Windows 11 22H2+ | DWM Desktop Acrylic | 需要窗后其他窗口内容；透明链正确；系统策略允许 | 微软文档、库源码；运行/性能未验证 |
| Windows 11 | Mica，作为不同视觉选项 | 接受壁纸基础层语义；按具体 API 选择版本条件 | 微软文档；产品选择未决 |
| Windows 10 / 早期 Windows 11 | 库的 Accent/特定 DWM 分支 | build、策略、窗口样式和性能逐项验证 | 库源码/已知风险；建议列实验性候选 |
| KDE X11 | `_KDE_NET_WM_BLUR_BEHIND_REGION` | compositing、支持声明、blur effect、正确物理像素区域 | KWindowSystem/KWin 源码；GTK 集成未验证 |
| KDE Wayland，旧协议广告存在 | `org_kde_kwin_blur_manager` v1 | GTK 同 display/surface；global 与对象生命周期 | XML 和 KWin Plasma/6.4 源码；运行未验证 |
| KDE Wayland，新协议能力存在 | `ext_background_effect_manager_v1` | 对应客户端协议、blur capability、目标版本 | KWin master 源码；完整客户端集成未验证 |
| 关闭效果/透明、能力缺失或其他桌面 | 主题实色外框 | 保留布局、对比度、点击与拖动行为 | 建议；跨平台运行未验证 |

## 尚未批准的选择与后续验收

1. Windows 是优先 Desktop Acrylic 的窗后模糊，还是优先 Mica 的长驻窗口性能/平台外观；是否接受 Windows 10 的库内部实现和性能限制。
2. macOS 经典材质的 material、失焦 state 与 appearance 策略；透明辅助功能关闭时采用的主题实色。
3. Linux 目标 KDE/Plasma 版本、X11 与 Wayland 两种后端的承诺范围，以及是否同时实现两代 Wayland 协议。
4. Linux 可能需要新增 GDK Wayland/X11 binding 或 Wayland 协议生成/FFI 依赖；是否需要取决于现有依赖实际导出的接口。新增依赖、共享状态或新架构均未获本轮授权。

后续运行验收至少需要：Windows 10 与 Windows 11 22H2+；macOS 13.4–25 与 26+；真实 KDE X11/Wayland。每个平台验证透明外框/opaque 内卡、前后窗口对比、亮暗主题、失焦、拖动/缩放、DPI/多屏、最大化与圆角、禁用透明/高对比或 Reduce Transparency，以及能力运行时移除。Linux 另验 surface 重建与 global/capability 变化；Windows 另验节能、远程会话与已知性能风险。

本轮只新增本研究记录与忽略目录内的证据输出。没有改生产代码、配置、依赖，没有编译、应用启动、Git stage/commit/branch/push 或发布。

## 一手来源与读取位置

Apple 文档读取了页面提供的 `.md` 全文；availability 使用其头部 metadata。SDK 核实了 `NSGlassEffectView.h`、`NSVisualEffectView.h` 与 `NSAccessibility.h` 的声明，SDK 作为补证，不以机器本地路径充当正式引用。

- [A1] Apple，[`NSGlassEffectView`](https://developer.apple.com/documentation/appkit/nsglasseffectview.md)，availability。
- [A2] Apple，[`NSVisualEffectView`](https://developer.apple.com/documentation/appkit/nsvisualeffectview.md)，Overview、Behind-window/In-window blending、Specifying the Effect Appearance；[`Material`](https://developer.apple.com/documentation/appkit/nsvisualeffectview/material-swift.enum.md)。
- [A3] window-vibrancy **0.6.0**，[`src/macos/internal.rs`](https://github.com/tauri-apps/window-vibrancy/blob/03fd627c4bbf740b4931b231095d08e27e838b91/src/macos/internal.rs)，`apply_vibrancy`，约 16–63 行。
- [A4] Apple SDK `NSVisualEffectView.h`，BlendingMode、State、material/state 声明；外观语义参见 [A2]。
- [A5] Apple，[`accessibilityDisplayShouldReduceTransparency`](https://developer.apple.com/documentation/appkit/nsworkspace/accessibilitydisplayshouldreducetransparency.md)，Discussion。
- [W1] Microsoft，[`Acrylic material`](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic)；已读取其[官方源文档](https://github.com/MicrosoftDocs/windows-dev-docs/blob/docs/hub/apps/design/style/acrylic.md)，约 36–85、99 行。
- [W2] Microsoft，[`Mica material`](https://learn.microsoft.com/en-us/windows/apps/design/style/mica)；已读取其[官方源文档](https://github.com/MicrosoftDocs/windows-dev-docs/blob/docs/hub/apps/design/style/mica.md)，约 13、38–52、72、112–114 行。
- [W3] Microsoft，[`DWM_SYSTEMBACKDROP_TYPE`](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwm_systembackdrop_type)，Constants、Requirements。
- [W4] Microsoft，[`DWMWINDOWATTRIBUTE`](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute)，`DWMWA_SYSTEMBACKDROP_TYPE`。
- [W5] Microsoft，[`DwmEnableBlurBehindWindow`](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/nf-dwmapi-dwmenableblurbehindwindow)，Remarks。
- [W6] window-vibrancy **0.6.0**，[`src/windows.rs`](https://github.com/tauri-apps/window-vibrancy/blob/03fd627c4bbf740b4931b231095d08e27e838b91/src/windows.rs)，`apply_blur`、`apply_acrylic`、`apply_mica` 和 build helpers。
- [W7] Microsoft，[`Apply Mica in Win32 desktop apps`](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/ui/apply-mica-win32)，Win32/WebView2 段落；[`Apply Mica or Acrylic`](https://learn.microsoft.com/en-us/windows/apps/develop/ui/system-backdrops)。
- [W8] window-vibrancy **0.6.0**，[`README.md`](https://github.com/tauri-apps/window-vibrancy/blob/03fd627c4bbf740b4931b231095d08e27e838b91/README.md)，Available functions，约 37–44 行；透明窗口示例条件约 30–33 行。
- [K1] KDE KWindowSystem，读取时 **master**，[`src/platforms/xcb/kwindoweffects.cpp`](https://github.com/KDE/kwindowsystem/blob/master/src/platforms/xcb/kwindoweffects.cpp)，`isEffectAvailable` 约 35–80 行、`enableBlurBehind`。
- [K2] KDE KWin，**Plasma/6.4**，读取快照 `4018344761abe6b7c5fb7b1c22374799f769cae6`，[`src/plugins/blur/blur.cpp`](https://github.com/KDE/kwin/blob/4018344761abe6b7c5fb7b1c22374799f769cae6/src/plugins/blur/blur.cpp)，manager 生命周期约 109–145 行，`supported` 约 374 行，`isActive` 约 856 行，`updateBlurRegion`。已核对 pinned 内容与先前读取的分支内容一致。
- [K3] KDE plasma-wayland-protocols，读取时 **master**，[`src/protocols/blur.xml`](https://github.com/KDE/plasma-wayland-protocols/blob/master/src/protocols/blur.xml)，完整 28 行。
- [K4] KDE KWindowSystem，读取时 **master**，[`src/platforms/wayland/windoweffects.cpp`](https://github.com/KDE/kwindowsystem/blob/master/src/platforms/wayland/windoweffects.cpp)，`BlurManager`、surface watchers、`isEffectAvailable`、blur 安装/清理；[`CMakeLists.txt`](https://github.com/KDE/kwindowsystem/blob/master/src/platforms/wayland/CMakeLists.txt) 中的协议生成输入。
- [K5] KDE KWin，读取时 **master**，快照 `fc9ced7980f9d59e46f6ca01e17a68708461a21b`，[`src/plugins/blur/blur.cpp`](https://github.com/KDE/kwin/blob/fc9ced7980f9d59e46f6ca01e17a68708461a21b/src/plugins/blur/blur.cpp)，约 166、189、432、911 行。已核对 pinned 内容与先前读取的分支内容一致。
- [K6] KDE KWin，与 [K5] 同一 master 快照，[`src/wayland/backgroundeffect_v1.cpp`](https://github.com/KDE/kwin/blob/master/src/wayland/backgroundeffect_v1.cpp)，约 18–74 行，background-effect manager 与 blur capability 实现。该读取不证明发行版版本覆盖。
- [G1] GNOME GTK，**gtk-3-24**，读取快照 `bac4a949b151eedada8bc60a97429d2176b0b28b`，[`gdk/wayland/gdkwaylanddisplay.h`](https://github.com/GNOME/gtk/blob/gtk-3-24/gdk/wayland/gdkwaylanddisplay.h)，约 48–49 行，`gdk_wayland_display_get_wl_display`。
- [G2] GNOME GTK，同一快照，[`gdk/wayland/gdkwaylandwindow.h`](https://github.com/GNOME/gtk/blob/gtk-3-24/gdk/wayland/gdkwaylandwindow.h)，约 48–49 行，`gdk_wayland_window_get_wl_surface`。原尝试的 GTK API 页面未取得，最终使用这两份已读上游头文件。
- [T1] Tauri **2.11.5**，`src/window/mod.rs::set_effects`、`src/vibrancy/mod.rs`；由协调者读取本地安装包源码核实。本研究没有以其他版本的在线文档替代此版本事实。

KDE 的 master 链接反映访问日读取状态，不能代替发布标签；后续实施须固定目标发行版和对应协议。DeepWiki 仅用于定位，没有作为关键结论依据。
