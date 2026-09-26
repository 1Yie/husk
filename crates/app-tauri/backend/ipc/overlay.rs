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

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use agent_ipc::UiEvent;

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
    .visible(false)
    .fullscreen(true);

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

    // Position bottom-right on whichever monitor the main window is on
    // (falls back to primary). `outer_position` is physical px; the HUD
    // `inner_size` is logical — convert through the scale factor so the
    // frame lands where the math says it should.
    let main = app.get_webview_window("main");
    let monitor = main
        .as_ref()
        .and_then(|w| w.current_monitor().ok().flatten())
        .or_else(|| main.as_ref().and_then(|w| w.primary_monitor().ok().flatten()));
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
    let verb = match (tool, action.trim()) {
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
    format!("正在{verb}")
}

/// Idle label the HUD falls back to between actions.
pub const HUD_IDLE: &str = "待命";

/// One self-contained DOM write — `eval`'d into the HUD webview so the
/// status lands even if the `__TAURI__` shim never injected into this
/// public-dir page (the failure mode that left the HUD stuck on idle).
pub fn hud_set_status_js(text: &str) -> String {
    let lit = serde_json::to_string(text).unwrap_or_else(|_| "\"待命\"".into());
    format!(
        "(function(){{var e=document.getElementById('action');if(e)e.textContent={lit};}})();"
    )
}

/// End the computer-control session: revoke the `computer` whitelist,
/// tear both overlay windows down, and resurface the main window.
/// Called from two places that must share one path —
///   • the HUD's "停止" button (`stop_computer_control` command), and
///   • the event forwarder when a turn reaches a terminal state while the
///     overlay is live (the model is done driving — auto-release).
/// MUST run on the main thread (touches windows).
pub fn end_computer_control(app: &AppHandle) {
    // Tell the kernel to drop the whitelist — the next `computer` call
    // falls back to Ask. Routed through `agent_cmd`'s normal path so the
    // gate write lands on the active session (the overlay has no idea
    // which session is controlling; kernel resolves `active()`).
    if let Ok(mgr) = app.state::<crate::kernel::KernelState>().0.lock() {
        if let Some(handle) = mgr.active() {
            if let Ok(mut gate) = handle.permissions.write() {
                gate.revoke_tool_for_session("computer");
            }
            let _ = handle.ui.send(UiEvent::SessionToolWhitelisted {
                tool_name: "computer".to_string(),
                stopped: true,
            });
        }
    }
    close_computer_overlay(app);
    // Bring the main window back — it was hidden behind the fullscreen
    // glow while the agent drove; on release the user should land back on
    // Husk, not a bare desktop with a dead overlay.
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.unminimize();
        let _ = main.show();
        let _ = main.set_focus();
    }
}

/// Tauri command for the frontend's "停止" button — emits a
/// `RevokeSessionTool{computer}` against the active session and closes
/// the overlay. Kept as a dedicated IPC (rather than the frontend going
/// through `agent_cmd`) so the overlay's close path is guaranteed to
/// fire even if the command pump is congested.
#[tauri::command]
pub fn stop_computer_control(app: AppHandle) -> Result<(), String> {
    end_computer_control(&app);
    Ok(())
}
