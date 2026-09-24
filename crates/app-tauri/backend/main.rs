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

fn main() {
    // Must run before GTK/libsoup init reads the env.
    #[cfg(target_os = "linux")]
    exempt_self_from_proxy();

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
            Ok(())
        })
        // Close (title-bar X, Alt+F4) hides to the tray instead of destroying
        // the window — `prevent_close` keeps the process alive, which is what
        // makes the single-instance restore path work. Real quit lives on the
        // tray menu.
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
            ipc::sessions::agent_session,
            ipc::sessions::mcp_probe,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Husk");
}
