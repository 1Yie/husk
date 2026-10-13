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
/// **Auto-detection:** every launch probes the session type and the GPU
/// vendor, then picks the whole env set in one shot — no restart, no
/// config needed:
///
/// | session | GPU     | backend  | dmabuf | extra env                      |
/// |---------|---------|----------|--------|--------------------------------|
/// | wayland | NVIDIA  | wayland  | on     | `__NV_DISABLE_EXPLICIT_SYNC=1` |
/// | wayland | other   | wayland  | on     | —                              |
/// | x11     | NVIDIA  | x11      | off    | GLX vendor pin                 |
/// | x11     | other   | x11      | on     | —                              |
///
/// `__NV_DISABLE_EXPLICIT_SYNC` works around WebKitGTK bug 324551: under
/// Wayland, egl-wayland enables linux-drm-syncobj on the toplevel surface
/// and WebKit then commits a wl_shm buffer with no acquire point, so KWin
/// kills the client ("Error 71 (protocol error)"). Under X11 the dmabuf
/// renderer's GBM allocation fails EINVAL on NVIDIA, so auto mode keeps
/// dmabuf off there (WebKitGTK falls back to its non-dmabuf GPU path).
///
/// **Override path:** `~/.config/husk/settings.toml` — user-level config
/// the user can flip even when the GUI is dead (edit the file, relaunch).
/// Explicit `renderer = "wayland" | "x11"` wins over session detection;
/// `gpu_acceleration = true` forces the dmabuf renderer on (pairing it
/// with `WEBKIT_DMABUF_RENDERER_DISABLE_GBM=1` under X11, where NVIDIA's
/// GBM backend rejects WebKitGTK's buffer flags); `false` forces pure
/// software rendering as a debugging floor.
///
/// **Window alpha:** the main window is created OPAQUE on Linux
/// (`tauri.linux.conf.json` flips the config's `transparent: true`). WebKitGTK
/// never paints a transparent GTK window on this stack: the surface comes up
/// with alpha=0 and WebKit's content never lands in it, so the compositor
/// shows the desktop (or the stale buffer) through the window and only the
/// rects WebKit damages on hover survive — highlights appear under the cursor
/// and vanish when it leaves. Reproduced with x11 and Wayland, software GL and
/// `WEBKIT_DISABLE_COMPOSITING_MODE=1`; `transparent: false` renders the whole
/// UI every time. The shell paints edge-to-edge (no rounded corners, no
/// shadow), so the window's alpha was never carrying any pixels.
#[cfg(target_os = "linux")]
fn linux_wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v == "wayland")
}

/// Any DRM card driven by NVIDIA (PCI vendor 0x10de). Skips connector
/// entries (`card0-DP-1` etc.) — only whole-card nodes carry `device/vendor`.
#[cfg(target_os = "linux")]
fn linux_nvidia_gpu() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return false;
    };
    entries.flatten().any(|e| {
        let name = e.file_name();
        let name = name.to_string_lossy();
        name.starts_with("card")
            && !name.contains('-')
            && std::fs::read_to_string(e.path().join("device/vendor"))
                .is_ok_and(|v| v.trim() == "0x10de")
    })
}

#[cfg(target_os = "linux")]
fn pin_linux_rendering_env() {
    let dev = agent_llm::config::DevConfig::load().unwrap_or_default();
    let nvidia = linux_nvidia_gpu();

    // GDK_BACKEND — auto-detect the session; settings.toml overrides.
    let wayland = match dev.renderer.as_deref() {
        Some("wayland") => true,
        Some("x11") => false,
        None => linux_wayland_session(),
        Some(other) => {
            eprintln!("husk: unknown dev.renderer {other:?} — auto-detecting");
            linux_wayland_session()
        }
    };
    std::env::set_var("GDK_BACKEND", if wayland { "wayland" } else { "x11" });

    // Hardware acceleration. `false` is the software floor for driver
    // debugging. `true` and the auto default both want the dmabuf renderer;
    // the only exception is auto mode on X11+NVIDIA, where dmabuf's GBM
    // allocation fails EINVAL and the non-dmabuf GLX path is healthier.
    match dev.gpu_acceleration {
        Some(false) => {
            // Software rendering — the safest floor, useful when debugging
            // driver-level GPU issues.
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
            std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
        }
        Some(true) | None => {
            if nvidia && !wayland && dev.gpu_acceleration.is_none() {
                // Auto mode on X11+NVIDIA: keep the classic non-dmabuf
                // GLX path; dmabuf-under-X11 floods `Failed to create GBM
                // buffer` (EINVAL) on this driver.
                std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
                std::env::set_var("__GLX_VENDOR_LIBRARY_NAME", "nvidia");
                return;
            }
            // dmabuf renderer on.
            std::env::remove_var("WEBKIT_DISABLE_DMABUF_RENDERER");
            std::env::remove_var("LIBGL_ALWAYS_SOFTWARE");
            if !nvidia {
                return;
            }
            std::env::set_var("__GLX_VENDOR_LIBRARY_NAME", "nvidia");
            if wayland {
                std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
            } else {
                // Explicit opt-in under X11: non-GBM dmabuf buffers avoid
                // NVIDIA GBM's EINVAL flood.
                std::env::set_var("WEBKIT_DMABUF_RENDERER_DISABLE_GBM", "1");
            }
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
/// Native ground colour for the main window, from the saved appearance.
/// Set before the window is shown, so the first frame the user sees is the
/// theme they chose — not the platform default (Linux pins `#fafafa`) that
/// the webview's own boot script can only correct a moment later.
///
/// Mirrors the frontend's rule: `dark` / `light` pin the mode, `system`
/// follows the OS. Dark uses the dark override when one is set.
fn main_window_ground(win: &tauri::WebviewWindow) -> tauri::webview::Color {
    let cfg = agent_kernel::load_appearance_settings();
    let dark = match cfg.theme_mode.as_str() {
        "dark" => true,
        "light" => false,
        _ => matches!(win.theme(), Ok(tauri::Theme::Dark)),
    };
    let hex = if dark {
        cfg.dark_background.as_deref().unwrap_or(&cfg.background)
    } else {
        &cfg.background
    };
    parse_hex_color(hex).unwrap_or(if dark {
        tauri::webview::Color(0x16, 0x16, 0x1a, 0xff)
    } else {
        tauri::webview::Color(0xfa, 0xfa, 0xfa, 0xff)
    })
}

/// `#rrggbb` (or `#rgb`) → an opaque colour. `None` for anything else, so a
/// malformed saved value falls back to the default instead of failing boot.
fn parse_hex_color(hex: &str) -> Option<tauri::webview::Color> {
    let h = hex.strip_prefix('#')?;
    let expand = |c: &str| u8::from_str_radix(&format!("{c}{c}"), 16).ok();
    let (r, g, b) = match h.len() {
        6 => (
            u8::from_str_radix(&h[0..2], 16).ok()?,
            u8::from_str_radix(&h[2..4], 16).ok()?,
            u8::from_str_radix(&h[4..6], 16).ok()?,
        ),
        3 => (expand(&h[0..1])?, expand(&h[1..2])?, expand(&h[2..3])?),
        _ => return None,
    };
    Some(tauri::webview::Color(r, g, b, 0xff))
}

fn fit_main_window(app: &tauri::App) {
    const BASE_W: f64 = 1280.0;
    const BASE_H: f64 = 840.0;
    const HEIGHT_FRACTION: f64 = 0.75;
    const ASPECT: f64 = BASE_W / BASE_H;

    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let reveal = |win: &tauri::WebviewWindow| {
        // Paint the saved theme before the first frame — see `main_window_ground`.
        let _ = win.set_background_color(Some(main_window_ground(win)));
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
        initialize_linux_gtk();
    }

    // Independent dev windows share file-based sessions; startup must not
    // terminate another window just because it uses the same executable.

    let builder = tauri::Builder::default();
    // Single instance: a second `husk` process exits immediately and the
    // running instance raises its main window instead — which is how a
    // tray-hidden window comes back when the app is launched again.
    //
    // Release only: dev processes intentionally remain independent.
    //
    // `dbus_id` is pinned rather than defaulted, so the name never depends on
    // `app.identifier` — the dev overlay swaps that, and a pinned id keeps the
    // release build's identity stable whichever config built it.
    #[cfg(not(debug_assertions))]
    let builder = if std::env::args().any(|arg| arg == "--new-window") {
        builder
    } else {
        builder.plugin(
            tauri_plugin_single_instance::Builder::new()
                .dbus_id("husk.ingstar.im")
                .callback(|app, _args, _cwd| focus_main_window(app))
                .build(),
        )
    };

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
                app.manage(
                    HuskTray {
                        app: app.handle().clone(),
                    }
                    .spawn()?,
                );
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
            app.manage(ipc::overlay::OverlayState::default());
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

/// Husk draws its own chrome. Use GTK's built-in theme by default instead
/// of loading a potentially incompatible desktop theme resource. Keep any
/// explicit GTK_THEME override and leave global desktop settings untouched.
#[cfg(target_os = "linux")]
fn initialize_linux_gtk() {
    use gtk::prelude::*;
    if std::env::var_os("GTK_THEME").is_none() {
        std::env::set_var("GTK_THEME", "Adwaita");
    }
    if gtk::init().is_err() {
        return;
    }
    if let Some(settings) = gtk::Settings::default() {
        let modules = settings
            .property::<Option<String>>("gtk-modules")
            .unwrap_or_default();
        // The borderless webview has no GTK menu bar for the optional KDE
        // appmenu integration. Exclude it only in this process, before the
        // first widget loads modules; retain other desktop integrations.
        let modules = modules
            .split(':')
            .filter(|m| *m != "appmenu-gtk-module")
            .collect::<Vec<_>>()
            .join(":");
        settings.set_property("gtk-modules", modules);
    }
}
