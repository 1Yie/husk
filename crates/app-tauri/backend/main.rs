//! `husk` — Tauri 2 desktop shell over the shared kernel wiring.
//!
//! `KernelState` (agent-kernel's `SessionManager`) owns the session actors and the
//! tagged `UiEvent` queue; `ipc/` exposes it as `agent_cmd` / `agent_session`
//! invokes plus an `agent://event` forwarder.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ipc;
mod kernel;

use kernel::KernelState;
#[cfg(not(target_os = "linux"))]
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri::{Manager, WindowEvent};

/// Bring the main window back to the foreground — used when a second instance
/// is launched (single-instance plugin), when the tray asks for the window,
/// and when close-to-tray hid it earlier.
fn focus_main_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// Dev-only: retire instances of this same binary left behind by earlier runs
/// (close-to-tray keeps them alive) so `cargo tauri dev` always comes up fresh.
///
/// Without this the single-instance handoff would win instead: the new dev
/// process exits, the leftover's window is raised, and that window's page was
/// served by a dev server this CLI has since killed — a stale or white window
/// under a `cargo tauri dev` that appears to have done nothing. Release keeps
/// the handoff, because there a relaunch *is* how a tray-hidden window returns.
#[cfg(all(debug_assertions, target_os = "linux"))]
fn retire_previous_dev_instances() {
    /// `/proc/<pid>/exe`, minus the `" (deleted)"` the kernel appends once the
    /// file has been relinked. Cargo rebuilds this binary on dev runs, so a
    /// leftover almost always points at a replaced file and a raw path compare
    /// would never match it.
    fn exe_path(pid: u32) -> Option<std::path::PathBuf> {
        let raw = std::fs::read_link(format!("/proc/{pid}/exe"))
            .ok()?
            .to_string_lossy()
            .into_owned();
        Some(std::path::PathBuf::from(
            raw.strip_suffix(" (deleted)").unwrap_or(raw.as_str()),
        ))
    }

    let self_pid = std::process::id();
    let Some(self_exe) = exe_path(self_pid) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        // Same binary path — a debug process from an earlier dev run, not the
        // release build (target/release/husk) or any other app.
        if pid == self_pid || exe_path(pid).as_deref() != Some(self_exe.as_path()) {
            continue;
        }
        // SAFETY: plain SIGTERM to a process just identified as this binary.
        unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    }
}

/// The bundled 32x32 app icon as raw RGBA — tray backends want pixels, not a
/// file, and `png` is already a dependency.
fn app_icon_rgba() -> (Vec<u8>, u32, u32) {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(
        &include_bytes!("../icons/32x32.png")[..],
    ));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().expect("tray icon png header");
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("tray icon png frame");
    buf.truncate(info.buffer_size());
    (buf, info.width, info.height)
}

/// Linux tray: a real StatusNotifierItem, because the libappindicator backend
/// behind Tauri's tray icon hands every click to the menu and cannot tell the
/// buttons apart. `ksni` wires left click to `Activate` and leaves the context
/// menu to the desktop shell, which is what a tray icon is supposed to do.
#[cfg(target_os = "linux")]
struct HuskTray {
    app: tauri::AppHandle,
}

#[cfg(target_os = "linux")]
impl ksni::Tray for HuskTray {
    fn id(&self) -> String {
        "husk".into()
    }

    fn title(&self) -> String {
        "Husk".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Husk".into(),
            ..Default::default()
        }
    }

    /// SNI pixmaps are ARGB32 in network byte order; `app_icon_rgba` yields RGBA.
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let (rgba, width, height) = app_icon_rgba();
        let data = rgba
            .chunks_exact(4)
            .flat_map(|px| [px[3], px[0], px[1], px[2]])
            .collect();
        vec![ksni::Icon {
            width: width as i32,
            height: height as i32,
            data,
        }]
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "显示主窗口".into(),
                activate: Box::new(|tray: &mut Self| focus_main_window(&tray.app)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "退出".into(),
                activate: Box::new(|tray: &mut Self| tray.app.exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }

    /// Left click on the tray icon.
    fn activate(&mut self, _x: i32, _y: i32) {
        focus_main_window(&self.app);
    }
}

/// Exempt the app's own origins from a proxy inherited from the shell env.
///
/// WebKitGTK resolves page loads through libsoup → libproxy: when the user
/// runs `cargo tauri dev` (or launches a release build) from a shell with
/// `http_proxy` set but no localhost exemption, the webview sends
/// `http://localhost:1420` — and in release `tauri.localhost` plus the IPC
/// bridge `ipc.localhost` — to the proxy, which tunnels them nowhere. The
/// page never arrives and the window renders blank with zero diagnostics.
///
/// Merging into `no_proxy`/`NO_PROXY` (the names libproxy actually reads)
/// fixes all proxy consumers without unsetting the user's proxy — env fetches
/// and IPC traffic that legitimately need it still go through.
#[cfg(target_os = "linux")]
fn exempt_self_from_proxy() {
    const EXEMPT: &[&str] = &[
        "localhost",
        "127.0.0.1",
        "::1",
        "tauri.localhost",
        "ipc.localhost",
    ];
    for var in ["no_proxy", "NO_PROXY"] {
        let existing = std::env::var(var).unwrap_or_default();
        let mut hosts: Vec<String> = existing
            .split(',')
            .map(|h| h.trim().to_string())
            .filter(|h| !h.is_empty())
            .collect();
        for h in EXEMPT {
            if !hosts.iter().any(|e| e.eq_ignore_ascii_case(h)) {
                hosts.push((*h).to_string());
            }
        }
        std::env::set_var(var, hosts.join(","));
    }
}

/// Pin the GTK/WebKitGTK rendering env before GTK init reads it.
///
/// Under Wayland + the proprietary NVIDIA driver, WebKitGTK's dmabuf
/// renderer hits a presentation-protocol conflict and the compositor
/// floods the log with `Failed to create GBM buffer` while the webview
/// renders broken or blank — and on some setups the process dies at
/// startup. Forcing the X11 backend (XWayland) plus disabling the dmabuf
/// renderer sidesteps the conflict without giving up GPU acceleration,
/// and `__GLX_VENDOR_LIBRARY_NAME` keeps GLX on the NVIDIA vendor library.
///
/// **Override path:** `~/.config/husk/settings.toml` — user-level config
/// the user can flip even when the GUI is dead (edit the file, relaunch).
/// Defaults below are the conservative NVIDIA-safe set; explicit opt-ins
/// (`renderer = "wayland"`, `gpu_acceleration = true`) are honoured.
#[cfg(target_os = "linux")]
fn pin_linux_rendering_env() {
    let dev = agent_llm::config::DevConfig::load().unwrap_or_default();

    // GDK_BACKEND — the conservative default is X11; `wayland` is opt-in.
    match dev.renderer.as_deref() {
        Some("wayland") => std::env::set_var("GDK_BACKEND", "wayland"),
        Some("x11") | None => std::env::set_var("GDK_BACKEND", "x11"),
        Some(other) => {
            eprintln!("husk: unknown dev.renderer {other:?} — falling back to x11");
            std::env::set_var("GDK_BACKEND", "x11");
        }
    }

    // Hardware-acceleration kill switch. Default is the NVIDIA-safe set
    // (dmabuf off so WebKitGTK falls back to its non-dmabuf GPU path).
    // `gpu_acceleration = false` additionally forces software GL; `true`
    // opts into the dmabuf renderer (the user accepts the NVIDIA breakage).
    match dev.gpu_acceleration {
        Some(true) => {
            // Opted into dmabuf — leave the var unset so WebKitGTK picks it.
            std::env::remove_var("WEBKIT_DISABLE_DMABUF_RENDERER");
            std::env::remove_var("LIBGL_ALWAYS_SOFTWARE");
            std::env::set_var("__GLX_VENDOR_LIBRARY_NAME", "nvidia");
        }
        Some(false) => {
            // Software rendering — the safest floor, useful when debugging
            // driver-level GPU issues.
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
            std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
        }
        None => {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
            std::env::set_var("__GLX_VENDOR_LIBRARY_NAME", "nvidia");
        }
    }
}

/// Size the main window to the monitor it opens on and center it.
///
/// The configured 1280×840 is the design baseline for a 1080-class
/// display, but tauri sizes are physical pixels and the compositor's
/// scale only exists at runtime: on a 150-200%-scaled Wayland panel the
/// fixed size lands cramped, and `GDK_BACKEND=x11` (see
/// `pin_linux_rendering_env`) flattens the reported scale factor to 1.0,
/// so the monitor's own `scale_factor` can't be trusted to recover it.
///
/// `work_area` is always physical pixels, so a fixed fraction of it
/// yields a consistent *logical* size at every scale without needing the
/// factor at all. The height drives: take ~75% of the work-area height,
/// then derive the width from the baseline 1280:840 aspect — a wide or
/// ultrawide monitor gets the same window, not a wider one, and a
/// 4K@200% panel lands back at roughly the baseline logical size. The
/// floor keeps the baseline when it fits, the ceiling clamps to the
/// work area so the window can never open larger than the usable
/// screen. A zero work area or a failed monitor probe leaves the
/// configured size alone.
///
/// `visible: false` in the config keeps the window off-screen until this
/// runs: under X11/XWayland an early-shown window flashes a black frame
/// (no compositing yet) and then visibly jumps when the resize lands, so
/// the reveal is deferred to one `show()` after the geometry is set.
fn fit_main_window(app: &tauri::App) {
    const BASE_W: f64 = 1280.0;
    const BASE_H: f64 = 840.0;
    const HEIGHT_FRACTION: f64 = 0.75;
    const ASPECT: f64 = BASE_W / BASE_H;

    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let reveal = |win: &tauri::WebviewWindow| {
        let _ = win.show();
        let _ = win.set_focus();
    };
    let monitor = win
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| win.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        // Geometry unknown — revealing at the configured size beats a
        // black window that never opens.
        reveal(&win);
        return;
    };

    let work = monitor.work_area();
    if work.size.width == 0 || work.size.height == 0 {
        reveal(&win);
        return;
    }
    let (work_w, work_h) = (work.size.width as f64, work.size.height as f64);
    let h = (HEIGHT_FRACTION * work_h)
        .max(BASE_H.min(work_h))
        .min(work_h);
    let w = (h * ASPECT).max(BASE_W.min(work_w)).min(work_w);
    let (w, h) = (w as u32, h as u32);

    // Position it ourselves rather than via `win.center()`: tao's
    // `center()` on Linux is `gtk_window_set_position(Center)`, which
    // asks the WM to center whatever size the window *currently* has.
    // The `set_size` below is still in flight when `center` runs, so
    // the WM centers the pre-resize 1280×840 frame — off-center under
    // X11/XWayland, right where this function matters.
    let x = work.position.x + ((work.size.width.saturating_sub(w)) / 2) as i32;
    let y = work.position.y + ((work.size.height.saturating_sub(h)) / 2) as i32;
    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
    let _ = win.set_size(tauri::PhysicalSize::new(w, h));
    reveal(&win);
}

fn main() {
    // Must run before GTK/WebKitGTK/libsoup init reads the env.
    #[cfg(target_os = "linux")]
    {
        pin_linux_rendering_env();
        exempt_self_from_proxy();
    }

    // Dev builds never inherit a previous run's process — see the function.
    #[cfg(all(debug_assertions, target_os = "linux"))]
    retire_previous_dev_instances();

    let builder = tauri::Builder::default();
    // Single instance: a second `husk` process exits immediately and the
    // running instance raises its main window instead — which is how a
    // tray-hidden window comes back when the app is launched again.
    //
    // Release only: under `cargo tauri dev` the handoff would raise a leftover
    // whose page died with the previous dev server (see
    // `retire_previous_dev_instances`).
    //
    // `dbus_id` is pinned rather than defaulted, so the name never depends on
    // `app.identifier` — the dev overlay swaps that, and a pinned id keeps the
    // release build's identity stable whichever config built it.
    #[cfg(not(debug_assertions))]
    let builder = builder.plugin(
        tauri_plugin_single_instance::Builder::new()
            .dbus_id("husk.ingstar.im")
            .callback(|app, _args, _cwd| focus_main_window(app))
            .build(),
    );

    builder
        // System clipboard access for image paste (see `paste_clipboard`).
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            // System tray. Close-to-tray keeps the process alive, so a click
            // on the tray — or, in release, a relaunch — brings this window
            // back instead of booting a second shell. `退出` is the only real
            // exit path now.
            #[cfg(target_os = "linux")]
            {
                use ksni::blocking::TrayMethods as _;
                app.manage(HuskTray { app: app.handle().clone() }.spawn()?);
            }
            #[cfg(not(target_os = "linux"))]
            {
                let show = MenuItemBuilder::with_id("show", "显示主窗口").build(app)?;
                let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;
                let menu = MenuBuilder::new(app).items(&[&show, &quit]).build()?;
                let icon = app
                    .default_window_icon()
                    .map(|icon| {
                        tauri::image::Image::new_owned(
                            icon.rgba().to_vec(),
                            icon.width(),
                            icon.height(),
                        )
                    })
                    .unwrap_or_else(|| {
                        let (rgba, width, height) = app_icon_rgba();
                        tauri::image::Image::new_owned(rgba, width, height)
                    });
                TrayIconBuilder::with_id("main")
                    .icon(icon)
                    .tooltip("Husk")
                    .menu(&menu)
                    // Left click raises the window; the menu opens on right click.
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| match event.id().as_ref() {
                        "show" => focus_main_window(app),
                        "quit" => app.exit(0),
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            focus_main_window(tray.app_handle());
                        }
                    })
                    .build(app)?;
            }

            // Boot the shared kernel manager, take its event receiver, and
            // forward it to the webview. `event_rx` is `std::sync::mpsc`
            // so it's moved, not cloned — the manager lives in `KernelState`.
            let (state, rx) = KernelState::spawn();
            let mgr = state.0.clone();
            app.manage(state);
            ipc::forwarder::spawn(app.handle().clone(), rx, mgr);

            // Adapt the configured 1280×840 to the monitor that actually
            // opened the window — see `fit_main_window`.
            fit_main_window(app);
            Ok(())
        })
        // Close (title-bar X, Alt+F4) hides to the tray instead of destroying
        // the window — `prevent_close` keeps the process alive, which is what
        // makes the single-instance restore path work. Real quit lives on the
        // tray menu. The computer-use overlay intentionally does NOT hide with
        // the main window: the whole point is that the user sees "AI is
        // controlling your screen" even when they've parked Husk itself —
        // the glow + HUD die with the process, or when the user clicks the
        // HUD's stop button.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            ipc::commands::agent_cmd,
            ipc::commands::get_ui_stats,
            ipc::commands::paste_clipboard,
            ipc::commands::check_update,
            ipc::overlay::stop_computer_control,
            ipc::sessions::agent_session,
            ipc::sessions::mcp_probe,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Husk");
}
