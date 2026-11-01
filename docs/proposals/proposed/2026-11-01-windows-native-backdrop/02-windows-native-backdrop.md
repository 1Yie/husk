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
- A Windows capability gate and event source: build 22621 minimum, reduced
  transparency, high contrast, battery saver, remote session, plus
  `WM_SETTINGCHANGE` and `WM_DWMCOMPOSITIONCHANGED` re-sync. Tauri does not
  expose the window procedure, so the subclass lives in this module.
- The frontend gate at `sidebar-glass.ts:58` becomes the platform capability
  from `01` instead of `isMac`, without touching the queue, the revision or the
  `settle(false)` path.

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
