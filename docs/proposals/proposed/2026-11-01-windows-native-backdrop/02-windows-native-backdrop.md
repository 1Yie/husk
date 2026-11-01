# 02 — Native backdrop and capability gate on Windows

## Scope

Give the Windows shell a real window backdrop behind the frame from `01`, and
make the command's `bool` an honest activation proof on that platform.

- `crates/app-tauri/backend/ipc/sidebar_glass/windows.rs`: extend the frame into
  the client area and set the system backdrop, checking every HRESULT.
  Desktop Acrylic (`DWMSBT_TRANSIENTWINDOW`), Mica (`DWMSBT_MAINWINDOW`) as the
  refused-type fallback.
- A target-gated `cfg(target_os = "windows")` dependency on the `windows` crate
  at the version tao already resolves (0.61.3), for the DWM calls, window-procedure
  subclassing and the system state reads.
- The transparency overrides in `backend/main.rs:395-420` and the window's
  background colour: transparent only while a backdrop is active, restored
  opaque on fallback, and no opaque frame before the first correct paint.
- The Windows capability gate and event source: build 22621 minimum, reduced
  transparency, high contrast, battery saver, remote session, plus
  `WM_SETTINGCHANGE` and `WM_DWMCOMPOSITIONCHANGED` re-sync. Tauri does not
  expose the window procedure, so the subclass lives in this module.
- The command's Windows branch. The frontend gate belongs to `01`, so this part
  touches no frontend file.

## Inputs

- The proposal `windows-native-backdrop`, acceptance criteria 1–6.
- `backend/ipc/sidebar_glass.rs` — the shared `Surface`, validation, clipping
  and the macOS dispatch shape to mirror.
- `backend/ipc/sidebar_glass/macos.rs` — the reference implementation for
  revision handling, the reduce-transparency path and the observer lifecycle.
- `backend/main.rs:190-210` (why the Linux window is opaque) and `:395-420`
  (`fit_main_window` and `main_window_ground`), `tauri.conf.json` window block.
- `tauri/src/window/mod.rs:2080` and `window-vibrancy` 0.6.0 — the recorded
  version that discards the backend result, which this part must not repeat.
- The research record `cross-platform-window-materials` for the build matrix and
  the refused paths.

## Must not touch

- The macOS module's behaviour and `index.css`'s coat value.
- Linux code paths: no Linux target compiles any of this, and Linux stays
  opaque until its own proposal.
- Layout files owned by `01`.
- The command signature, the surface validation and the frontend contract.

## Done condition

- The Windows target compiles with the new module and no new warnings, checked
  locally if the toolchain allows a Windows-target `cargo check`, otherwise on
  the owner's machine before any behaviour run.
- On Windows 11 22H2+ the acceptance criteria hold in a captured run: the
  desktop tints the frame, the inner surfaces stay opaque, resize/DPI/focus
  re-sync without a gap, and there is no translucent-but-unblurred state.
- Each disabling condition — a build below 22621, reduced transparency, high
  contrast, battery saver, a remote session, `DwmSetWindowAttribute` refusal —
  returns `false`, removes `data-sidebar-glass` and restores the solid frame.
- A machine where the platform is known unsupported issues no IPC from the
  frontend, and macOS behaviour is unchanged.

## Result

- `backend/ipc/sidebar_glass/windows.rs` carries the material, the gate and the
  window-procedure subclass; `sidebar_glass.rs` dispatches to it and every other
  target keeps `Ok(false)`.
- The client area is kept unpainted while a material is live: tao's erase fill is
  skipped and the window and WebView2 ground move between the theme colour and
  transparency, so a fallback is never left translucent.
- `cargo check -p husk` still passes on macOS with only its pre-existing
  `log_path` warning, and `cargo test -p husk sidebar_glass` passes.
- The Windows module type-checks against the real crates: a throwaway crate that
  compiles the same file for `x86_64-pc-windows-msvc` passes `cargo check` and
  `cargo clippy` with no warnings. The app crate itself cannot be built for
  Windows here, because `aws-lc-sys` needs the Windows SDK headers.
- **Behaviour is unverified.** Nothing here proves the material composites on a
  real window, that Acrylic degrades to Mica when refused, or that each policy
  branch returns to solid. Those are the acceptance runs on Windows 11 hardware,
  and until they pass this part stays unshipped.

## Hardware run once a Windows 11 22H2+ machine is available

Build and start the shell with `cargo tauri dev` from `crates/app-tauri`, then
walk the six acceptance criteria of the proposal. What each branch looks like
from the outside:

1. Default machine: the desktop colour shows through the frame and the sidebar,
   `data-sidebar-glass` is present on `<html>`, and the inner work surfaces, chat
   card, Dock and terminal stay opaque. If the frame is solid here, the material
   was refused — check that the build is 22621 or newer.
2. Resize the window, drag it between two monitors with different scaling, enter
   and leave fullscreen, and focus another window: the frame must not show a
   stale rectangle, a black band or a flash of the theme colour.
3. Turn Settings → Personalisation → Transparency effects off and on: the frame
   must go solid and back without a restart.
4. Repeat with high contrast, with battery saver, and over a remote session; each
   must end solid. Under `cargo tauri dev`, `Get-Latest` on the debug console
   shows whether the command answered `true`.
5. On a Windows 10 machine the frame must be solid and the shell must issue no
   IPC at all.
6. macOS behaviour must be unchanged on the same build.

Any failure is evidence for a follow-up commit on this branch; the parts that
fail stay out of `main` until they pass.

### What the sources already rule out, and what they leave open

Checked in the pinned crates rather than assumed:

- **Hit testing and the client area stay with tao.** `WM_NCCALCSIZE` (tao
  `platform_impl/windows/event_loop.rs:2123`) and `WM_NCHITTEST` (`:2178`, which
  is what gives an undecorated window its resize borders) are handled by tao's
  own window procedure; this module delegates every message it does not handle
  to that procedure, so extending the frame into the client area does not take
  the drag region or the edge resize with it. Watch for it anyway: the first
  drag of an undecorated window is the cheapest way to notice.
- **The legacy blur-behind call is a live variable.** tao calls
  `DwmEnableBlurBehindWindow` with an *empty* region whenever the window is
  created `transparent` (tao `platform_impl/windows/window.rs:1284-1296`), which
  is the pre-Windows-11 way to get a see-through client area. That is an older
  mechanism than a system backdrop, and which one wins is not documented. If the
  material does not appear, the next thing to try is creating the main window
  without `transparent` on Windows and letting the system backdrop plus the
  WEBVIEW2 alpha-0 background carry it; the current config reaches Windows
  through the `transparent` path because there is no Windows platform config.
- **Still unresolved by reading:** whether WebView2's own composited surface is
  opaque here, whether an accepted `DWMWA_SYSTEMBACKDROP_TYPE` renders as a
  material or as solid under each policy combination, and whether the acrylic to
  mica fallback actually triggers. None of that can be read out of the source.

### Capture with each run

Save the answer of the command, the window state and a screenshot together:

- `ver` (build number), whether the session is remote, and the power state;
- the `data-sidebar-glass` attribute on `<html>` and the value the command
  returned;
- whether the frame paints the desktop through it, and whether the inner
  surfaces stayed opaque;
- which of `DWMSBT_TRANSIENTWINDOW` and `DWMSBT_MAINWINDOW` the answer implies,
  by looking at whether the frame looks milky (mica) or paper-like (acrylic).
