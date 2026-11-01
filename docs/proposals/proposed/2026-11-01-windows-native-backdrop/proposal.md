# Enable the native window backdrop on Windows

Accepted: 2026-11-01

## Problem

The frameless shell only gets a native backdrop on macOS, so the 「外层玻璃」 the
owner asked for does not exist on Windows:

- `crates/app-tauri/backend/ipc/sidebar_glass.rs:4` compiles the native module
  only for macOS, and the command returns `Ok(false)` on every other target
  (`sidebar_glass.rs:92`).
- `crates/app-tauri/frontend/src/app/layout/main-layout/sidebar-glass.ts:58`
  returns before calling that command unless `isMac && isTauri()`, so the
  `html[data-sidebar-glass]` chain in `index.css:150-166` never activates.
- Windows has no frame for a backdrop to sit behind. The inset rounded shell is
  gated on `isMac` per component — the padded `main-workspace-frame`
  (`main-layout/index.tsx:163`), the transparent title bar
  (`components/title-bar/index.tsx:422`), the inset chat card
  (`chat/pages/chat/index.tsx:413`), the session sidebar, the Dock, the terminal
  panel and the settings page — so Windows paints an edge-to-edge shell with a
  bordered sidebar and `--husk-panel`, which is the solid look the request is
  about.
- Transparent configuration alone is not a backdrop. `backend/main.rs:416`
  paints the main window with an opaque theme colour on every platform; on
  Windows that colour reaches `WM_ERASEBKGND` (tao
  `platform_impl/windows/event_loop.rs:1104-1119`) and WebView2's
  `SetDefaultBackgroundColor`, where wry forces alpha to 255 for any non-zero
  value (wry `webview2/mod.rs:1810-1818`). Tauri's
  `WebviewWindow::set_background_color` writes both layers
  (`tauri/src/webview/webview_window.rs:2284`), so an active backdrop has to
  keep this path transparent and restore it on fallback.
- `crates/app-tauri/Cargo.toml` declares no Windows crate, so the DWM calls
  need a new direct dependency. `windows` 0.61.3 is already in the graph
  through tao, so this adds features rather than a second copy.

The owner's request: enable the same outer-glass effect on Windows, keeping the
opaque inner work surfaces and the current macOS geometry. The option space,
the platform API facts and the version matrix were investigated in the record
`cross-platform-window-materials`; this proposal cites it instead of restating
those findings.

## Proposal

Keep one contract and add a Windows implementation behind it. The frontend
contract, the CSS chain and the macOS geometry stay as they are; Windows gains
the frame the chain expects and a translated backdrop behind it.

### Shared contract (unchanged)

`sidebar_glass_update(main window only, revision, surface | null) -> bool`
keeps validating geometry, dropping stale revisions and returning whether the
backdrop is really active. `true` is the only thing allowed to switch on
`data-sidebar-glass`; anything else stays solid, so an unsupported machine is
never left translucent-but-unblurred. The per-document queue, the reduced
defaults and the `settle(false)` failure path in `sidebar-glass.ts:24-58` are
reused as they are.

The coat alpha currently lives in one place (`index.css:163`, 68%) because
macOS is the only producer. Windows measures its own composited result and, if
it needs a different coat, reads a platform value from the same owner rather
than adding a second global constant.

### Framed shell on Windows (`01`)

The glass needs the inset rounded shell. The `isMac` layout gates become a
platform capability so Windows renders the same frame, gap and radius with
opaque inner surfaces, and macOS output stays exactly as it is. Shortcut
labels, traffic-light placement and the resize grips keep their existing
per-platform behaviour. Linux stays edge-to-edge until its own work.

### Native backdrop on Windows (`02`)

- Implement `backend/ipc/sidebar_glass/windows.rs` that extends the frame into
  the client area and sets a system backdrop, checking every HRESULT and
  returning `Err`/`false` instead of assuming success (the recorded
  `set_effects` path discards the result — `tauri/src/window/mod.rs:2080` — so
  it cannot be the activation proof). Dependency: the `windows` crate at the
  version tao already resolves, target-gated on `cfg(target_os = "windows")`.
- Material: Desktop Acrylic (`DWMSBT_TRANSIENTWINDOW`), which is the "see the
  other windows behind" look the macOS build has, with Mica
  (`DWMSBT_MAINWINDOW`) as the fallback when the acrylic type is refused.
- Gate on the Windows build number: system backdrops exist from Windows 11
  22H2 (build 22621). Windows 10 and older builds keep the solid fallback; the
  Accent policy path is not adopted (see Alternatives).
- Keep the window and WebView transparent only while a backdrop is active:
  skip the opaque `set_background_color` for the main window on Windows, keep
  tao's own background colour unset, and let WebView2's default background stay
  at alpha 0. Fallback restores the opaque ground so no frame is ever
  translucent without blur.
- Return `false` — and stay solid — when the system asks for reduced
  transparency, high contrast or battery saver, and on remote sessions. The
  per-session preference is read through Win32 state rather than assumed from
  the `DwmSetWindowAttribute` result, because the system may accept the
  attribute and still composite it solid.
- Re-sync on the same events the macOS path already reacts to, from a Windows
  source: subclass the window procedure for `WM_SETTINGCHANGE` and
  `WM_DWMCOMPOSITIONCHANGED`, and follow power and session state. Tauri does
  not expose the window procedure, so this source is new code in the same
  module, not a reuse of the AppKit observers.

### Out of scope

- **Linux (KDE X11 and Wayland).** A compositor blur only shows through a
  transparent window, and the main window is deliberately opaque on Linux. The
  research record `webkitgtk-transparent-window` owns that question, its open
  experiments and what each result decides; until it concludes, a Linux part
  could only ever answer `false`. Nothing in this proposal compiles on Linux.
- **macOS 13.4–25 classic material.** Separate request; the current macOS 26+
  path is untouched.

## Alternatives

- **Stay macOS-only.** Rejected by the owner's request; it also leaves the same
  feature looking different per platform.
- **Let only the sidebar show the material, keeping the current Windows
  layout.** Rejected: the shell the owner asked for is one outer frame wrapping
  separate rounded work surfaces, so a translucent full-height sidebar would be
  a second shell design rather than the same one on another platform. It is the
  cheaper option if the framed shell is later refused for Windows.
- **Adopt `window-vibrancy` for Windows.** Rejected as the default: the
  recorded review found it discards the backend result on Windows, which cannot
  produce the `active` truth this contract needs, and it would add a dependency
  for behaviour we must verify ourselves anyway. It remains a fallback if the
  direct calls prove unmaintainable.
- **Windows 10 Accent policy for every Windows build.** Rejected: it is an
  undocumented path with its own failure modes, and it would put a second
  material mechanism behind the same attribute with worse resize behaviour.
- **CSS-only translucency without a native backdrop.** Rejected: CSS can only
  let the app's own content show through, not the desktop, so it would look
  like a different product from the macOS build.

## Acceptance criteria

On Windows 11 22H2+ hardware, with the same evidence style the macOS unit
already uses:

1. The shell renders the same inset, radius and gap as macOS, and
   `data-sidebar-glass` is present once a backdrop is active; the desktop
   colour visibly tints the frame, while inner work surfaces, cards, Dock and
   terminal stay opaque.
2. Text and controls keep at least 4.5:1 against the composited background in
   light, dark and a user font, measured from a real capture rather than from
   token values.
3. Resize, DPI/backing change, maximised/restored and focus changes re-sync the
   backdrop without a visible gap or a stale rectangle, and window creation
   never flashes an opaque frame before the first correct paint.
4. Disabling the system effect (reduced transparency, high contrast, battery
   saver, remote session, a build below 22621) returns `false`, removes the
   attribute and restores the solid panel; no IPC is issued when the platform
   is known unsupported.
5. Unsupported machines never show a translucent-but-unblurred shell, and the
   inner surfaces never become translucent.
6. Input, menus, drag regions, panel reveal timing and the opaque fallback keep
   passing their current browser and native checks, and the existing macOS
   behaviour is unchanged.

## Risks

- The Windows path cannot be verified in this environment; it needs the
  owner's machine. Compilation of the Windows target is checkable locally,
  behaviour is not, so a change that is not run there stays inactive code that
  returns `false`.
- The framed shell changes the Windows appearance even where no backdrop is
  available. That is intended — one shell design, solid frame where the
  material is unavailable — but it is a visible change on Windows 10 as well.
- Undocumented or build-specific behaviour may change between Windows releases;
  the capability check has to be conservative and is the thing that decides
  whether the shell stays solid.
- A blur behind a full-window WebView costs GPU time during drag and resize;
  the recorded Acrylic drag/scale warning applies to this path. If the measured
  frame budget regresses, the backdrop stays off.
- Reusing the single coat constant may be wrong once DWM adds its own tint; the
  per-platform value must stay in one owner.

## Checklist

- `01` — framed shell parity on Windows: replace the `isMac` layout gates with
  a platform capability. Verifiable locally.
- `02` — native backdrop and capability gate: `windows.rs`, the transparency
  overrides and the Windows event source. 02 after 01, and needs Windows 11
  hardware for acceptance.

## Decisions taken

- Windows 11 22H2+ only; Windows 10 keeps the solid frame.
- Desktop Acrylic with Mica as the fallback material.
- The framed shell is part of this request, so Windows and macOS share one
  shell design.
- The target-gated `windows` and `windows-version` dependencies are approved;
  both are already in the graph, so the build gains no new crate.

## State

Both parts are implemented on `task/windows-backdrop`. `01` is verified here;
`02` compiles for the Windows target and its behaviour waits for a run on
Windows 11 hardware, which this machine cannot provide.
