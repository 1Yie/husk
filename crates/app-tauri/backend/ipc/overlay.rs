//! The computer-use overlay — two Tauri windows painted over the desktop
//! while the agent has synthetic-input permission:
//!
//! - `computer-glow` — a fullscreen, transparent, click-through frame
//!   rendering the colored edge gradient (Apple-Intelligence-style "this
//!   screen is being watched" cue). `set_ignore_cursor_events(true)` so
//!   the user's pointer passes straight through to whatever is under it.
//!
//! - `computer-hud` — a small always-on-top HUD in a screen corner. It
//!   tells the user *what* is happening ("AI is controlling your
//!   screen"), shows the live action, and carries the **stop** button
//!   that revokes the session whitelist.
//!
//! Both are spawned on `UiEvent::SessionToolWhitelisted{stopped:false}`
//! for `tool_name == "computer"` and destroyed on `stopped:true` (or on
//! app close). They are NOT in `tauri.conf.json`'s `windows` array —
//! spawned imperatively so they can be brought up / down mid-session.

use agent_ipc::UiEvent;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

/// Logical size of the HUD bubble — one line of text plus the stop
/// button. Height hugs the single-line content so there's no dead
/// padding above/below it.
const HUD_W: f64 = 300.0;
const HUD_H: f64 = 38.0;
/// Margin from the screen edge — matches the 12px breathing room the
/// glow frame itself uses.
const HUD_MARGIN: f64 = 12.0;

/// Spawn (or re-show) the two overlay windows. Idempotent — calling
/// `open_computer_overlay` on an already-open overlay repositions the
/// HUD to the current monitor instead of duplicating it.
pub fn open_computer_overlay(app: &AppHandle) -> Result<(), String> {
    spawn_glow(app)?;
    spawn_hud(app)?;
    Ok(())
}

/// Tear both windows down. Idempotent — closing a missing window is a
/// no-op so callers don't need to track whether the overlay is live.
pub fn close_computer_overlay(app: &AppHandle) {
    for label in ["computer-glow", "computer-hud"] {
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.close();
        }
    }
}

/// The fullscreen click-through frame — owns the colored edge gradient.
/// Click-through is the whole point: a normal window would eat every
/// click the agent (and the user) tried to make.
fn spawn_glow(app: &AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("computer-glow") {
        let _ = w.show();
        let _ = w.set_ignore_cursor_events(true);
        return Ok(());
    }

    // Wayland layer-shell covers the monitor by anchoring all four edges. A
    // `fullscreen()` request on the same window makes GTK treat it as a
    // toplevel and the layer surface stops covering the screen, so it is only
    // set on the toplevel path.
    #[cfg(target_os = "linux")]
    let layer = crate::ipc::layer_shell::available();
    #[cfg(not(target_os = "linux"))]
    let layer = false;

    let builder = WebviewWindowBuilder::new(
        app,
        "computer-glow",
        WebviewUrl::App("overlay-glow.html".into()),
    )
    .title("Husk — computer control")
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .visible(false);
    let builder = if layer {
        builder
    } else {
        builder.fullscreen(true)
    };

    // `.transparent` is compiled out on macOS (`#[cfg(any(not(macos),
    // macos-private-api))]`) — calling it unconditionally breaks the macOS
    // cross-compile. Real overlay transparency there needs the
    // `macos-private-api` Tauri feature; until that's enabled the glow is
    // non-transparent on macOS but the window still builds and runs.
    #[cfg(not(target_os = "macos"))]
    let builder = builder.transparent(true);

    let win = builder
        .build()
        .map_err(|e| format!("spawn glow window: {e}"))?;

    // Wayland: place it as a full-monitor layer surface. The window is built
    // `visible(false)`, so it is still unrealized — the one moment
    // `init_for_window` accepts (see layer_shell.rs).
    #[cfg(target_os = "linux")]
    if layer {
        let gtk_win = win
            .gtk_window()
            .map_err(|e| format!("glow gtk window: {e}"))?;
        crate::ipc::layer_shell::full_screen(&gtk_win);
    }

    // The entire surface must pass clicks through — this window exists
    // only to paint, not to interact.
    //
    // Order matters: the window is built `visible(false)`, so it has no GDK
    // window yet. On Linux, `set_ignore_cursor_events` reaches into
    // `window.window().unwrap()` (tao `CursorIgnoreEvents`) — calling it
    // BEFORE `show()` unwraps `None` and aborts. Show first, then mark it
    // click-through once it's realized.
    win.show().map_err(|e| e.to_string())?;
    win.set_ignore_cursor_events(true)
        .map_err(|e| format!("glow ignore_cursor_events: {e}"))?;
    Ok(())
}

/// The HUD bubble — bottom-right of the primary monitor. Interactive so
/// the "stop" button is clickable, always-on-top so the agent (or the
/// user) can't lose it under another window.
fn spawn_hud(app: &AppHandle) -> Result<(), String> {
    let win = match app.get_webview_window("computer-hud") {
        Some(w) => w,
        None => WebviewWindowBuilder::new(
            app,
            "computer-hud",
            WebviewUrl::App("overlay-hud.html".into()),
        )
        .title("Husk — controlling")
        .decorations(false)
        // NOT transparent — the HUD is a solid card filling its window, so
        // alpha buys nothing (and `.transparent` doesn't even exist on macOS).
        // Opaque is the default anyway, so there's no call to make.
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .visible(false)
        .inner_size(HUD_W, HUD_H)
        .build()
        .map_err(|e| format!("spawn hud window: {e}"))?,
    };

    // Wayland: a layer surface anchored bottom-right. The window is still
    // unrealized here (built `visible(false)`), which layer-shell requires.
    // The toplevel placement below is ignored on Wayland, so it is skipped.
    #[cfg(target_os = "linux")]
    if crate::ipc::layer_shell::available() {
        let gtk_win = win
            .gtk_window()
            .map_err(|e| format!("hud gtk window: {e}"))?;
        // The layer surface takes its size from the request, not from
        // `inner_size` alone — pin it to the logical HUD size.
        use gtk::prelude::WidgetExt as _;
        gtk_win.set_size_request(HUD_W as i32, HUD_H as i32);
        crate::ipc::layer_shell::corner_hud(&gtk_win, HUD_MARGIN as i32);
        win.show().map_err(|e| e.to_string())?;
        return Ok(());
    }

    // Position bottom-right on whichever monitor the main window is on
    // (falls back to primary). `outer_position` is physical px; the HUD
    // `inner_size` is logical — convert through the scale factor so the
    // frame lands where the math says it should.
    let main = app.get_webview_window("main");
    let monitor = main
        .as_ref()
        .and_then(|w| w.current_monitor().ok().flatten())
        .or_else(|| {
            main.as_ref()
                .and_then(|w| w.primary_monitor().ok().flatten())
        });
    if let Some(monitor) = monitor {
        let scale = monitor.scale_factor();
        let work = monitor.work_area();
        let hud_w_px = (HUD_W * scale) as i32;
        let hud_h_px = (HUD_H * scale) as i32;
        let margin_px = (HUD_MARGIN * scale) as i32;
        let x = work.position.x + work.size.width as i32 - hud_w_px - margin_px;
        let y = work.position.y + work.size.height as i32 - hud_h_px - margin_px;
        let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
    }

    win.show().map_err(|e| e.to_string())?;
    // The glow is also `always_on_top` and fullscreen — the HUD must be
    // raised ABOVE it or the click-through region still shadows the stop
    // button under XWayland. Re-asserting always-on-top + focus pops the
    // HUD to the front of the always-on-top layer so its clicks land.
    let _ = win.set_always_on_top(true);
    let _ = win.set_focus();
    Ok(())
}

/// Map a `computer`/`screenshot` tool action to the Chinese status the
/// HUD shows under "控制中" — kept Rust-side so the label can be pushed
/// into the HUD webview with `eval` (no `__TAURI__` bridge required).
pub fn hud_action_label(tool: &str, action: &str) -> String {
    // `computer` previews read `<action> <detail>` (see `describe`): the verb
    // keys off the action alone, the detail is shown after it.
    let action = action.trim();
    let (action, detail) = match (tool, action.split_once(' ')) {
        ("computer", Some((a, d))) => (a, Some(d)),
        _ => (action, None),
    };
    let verb = match (tool, action) {
        ("screenshot", _) => "截图",
        ("computer", "left_click") => "点击",
        ("computer", "right_click") => "右键点击",
        ("computer", "middle_click") => "中键点击",
        ("computer", "double_click") => "双击",
        ("computer", "mouse_move") => "移动光标",
        ("computer", "left_click_drag") => "拖拽",
        ("computer", "type") => "输入",
        ("computer", "key") => "按键",
        ("computer", "hold_key") => "按住",
        ("computer", "scroll") => "滚动",
        ("computer", "wait") => "等待",
        ("computer", "cursor_position") => "读取光标",
        ("computer", "launch") => "启动应用",
        (_, other) if !other.is_empty() => other,
        _ => "操作",
    };
    match detail {
        Some(d) if d.chars().count() > 40 => {
            format!("正在{verb} {}…", d.chars().take(40).collect::<String>())
        }
        Some(d) => format!("正在{verb} {d}"),
        None => format!("正在{verb}"),
    }
}

/// Idle label the HUD falls back to between actions.
pub const HUD_IDLE: &str = "待命";

/// One self-contained DOM write — `eval`'d into the HUD webview so the
/// status lands even if the `__TAURI__` shim never injected into this
/// public-dir page (the failure mode that left the HUD stuck on idle).
pub fn hud_set_status_js(text: &str) -> String {
    let lit = serde_json::to_string(text).unwrap_or_else(|_| "\"待命\"".into());
    format!("(function(){{var e=document.getElementById('action');if(e)e.textContent={lit};}})();")
}

/// The session that currently holds the desktop. Captured when the grant
/// lands, with the handles needed to stop it — so "停止" never has to take the
/// manager lock (which an MCP handshake can hold for seconds) and never acts
/// on whichever session happens to be active *now*.
pub struct Controlled {
    pub root: String,
    pub session: i64,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    permissions: std::sync::Arc<std::sync::RwLock<agent_kernel::permissions::PermissionGate>>,
    ui: agent_kernel::channels::UiSink,
}

impl Controlled {
    pub fn of(root: &str, session: i64, h: &husk_host::session_manager::SessionHandle) -> Self {
        Self {
            root: root.to_string(),
            session,
            cancel: h.cancel.clone(),
            permissions: h.permissions.clone(),
            ui: h.ui.clone(),
        }
    }

    pub fn is(&self, root: &str, session: i64) -> bool {
        self.session == session && self.root == root
    }
}

/// Tauri state: who is driving the desktop right now (`None` = nobody). One
/// at a time — a newer grant takes over the overlay.
#[derive(Default)]
pub struct OverlayState(pub std::sync::Mutex<Option<Controlled>>);

/// Tear both windows down and bring the main window back — it was hidden
/// behind the fullscreen glow while the agent drove, so on release the user
/// should land back on Husk, not a bare desktop with a dead overlay.
/// MUST run on the main thread (touches windows).
pub fn close_and_restore(app: &AppHandle) {
    close_computer_overlay(app);
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.unminimize();
        let _ = main.show();
        // tao drops `set_focus` while the window still reads as minimized, and
        // `unminimize` only takes effect on the next loop turn. Focus once the
        // restore has landed, or a finished turn leaves Husk minimized.
        #[cfg(target_os = "linux")]
        {
            let main = main.clone();
            gtk::glib::idle_add_local_once(move || {
                let _ = main.set_focus();
            });
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = main.set_focus();
        }
    }
}

/// The user pressed 停止: end the grant on the session that holds it, abort
/// its in-flight turn (a revoke alone only affects the *next* call — the one
/// already running, and every queued one, would still go through), and close
/// the overlay. No-op for the kernel half when nobody is controlling.
pub fn stop_controlled(app: &AppHandle) {
    let taken = app
        .state::<OverlayState>()
        .0
        .lock()
        .ok()
        .and_then(|mut g| g.take());
    if let Some(c) = taken {
        if let Ok(mut gate) = c.permissions.write() {
            gate.revoke_grants("computer");
        }
        c.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = c.ui.send(UiEvent::SessionToolWhitelisted {
            tool_name: "computer".to_string(),
            stopped: true,
        });
    }
    close_and_restore(app);
}

/// Tauri command for the HUD's "停止" button. A dedicated IPC (not `agent_cmd`)
/// so the stop path never queues behind a congested command pump.
#[tauri::command]
pub fn stop_computer_control(app: AppHandle) -> Result<(), String> {
    stop_controlled(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hud_label_uses_the_action_and_shows_the_detail() {
        assert_eq!(hud_action_label("screenshot", ""), "正在截图");
        assert_eq!(hud_action_label("computer", "left_click"), "正在点击");
        assert_eq!(
            hud_action_label("computer", "left_click (12, 34)"),
            "正在点击 (12, 34)"
        );
        assert_eq!(
            hud_action_label("computer", "launch \"kwrite a.txt\""),
            "正在启动应用 \"kwrite a.txt\""
        );
        let long = format!("type \"{}\"", "x".repeat(100));
        assert!(hud_action_label("computer", &long).ends_with('…'));
    }
}
