//! `agent_cmd` — the single invoke for all `UiCommand`s.
//!
//! One command, the enum carries the intent. `Cancel`, `Steer` and `ToolDecision`
//! bypass the command pump (direct flag / channel) so they are not queued behind a
//! running turn.

use agent_ipc::{UiCommand, UiEvent};
use tauri::State;

use crate::kernel::KernelState;

/// `paste_clipboard` — read the *system* clipboard from Rust, for the composer's
/// Ctrl+V and its context menu.
///
/// WebKitGTK's own `paste` event is not a dependable carrier for images: pasting
/// into a plain `<textarea>` can deliver no `clipboardData` at all. The bytes are
/// reachable here regardless.
///
/// `kind` = `"image"` (default) stages the clipboard image as an attachment;
/// `"text"` returns the clipboard string. An image-less clipboard is `null`, not an
/// error — the caller then leaves the browser's own paste alone.
///
/// `async` so `read_image` does not run on the main thread (arboard can deadlock on
/// Linux when the clipboard holds data this app copied).
#[tauri::command]
pub async fn paste_clipboard(
    app: tauri::AppHandle,
    state: State<'_, KernelState>,
    kind: String,
) -> Result<serde_json::Value, String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let clipboard = app.clipboard();

    if kind == "text" {
        return Ok(match clipboard.read_text() {
            Ok(text) if !text.is_empty() => serde_json::json!({ "kind": "text", "text": text }),
            _ => serde_json::Value::Null,
        });
    }

    let Ok(image) = clipboard.read_image() else {
        return Ok(serde_json::Value::Null);
    };
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 {
        return Ok(serde_json::Value::Null);
    }
    let png = crate::ipc::sessions::encode_png(image.rgba(), width, height)?;
    // Lock only long enough to copy the root — never across the clipboard or
    // file work above.
    let root = {
        let mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.workspace_root.clone()
    };
    if root.as_os_str().is_empty() {
        return Err("没有打开的工作区".into());
    }
    crate::ipc::sessions::stage_clipboard_image(&png, &root)
}

/// `get_ui_stats` — delivery counters for every live session's UI event
/// queue. The status chip reads them so a lagging consumer (dropped deltas,
/// deepening queue) is visible instead of silent. Control events are never
/// dropped, so `dropped > 0` always means *text* was lost.
#[tauri::command]
pub fn get_ui_stats(
    state: State<'_, KernelState>,
) -> Result<agent_kernel::channels::UiStatsSnapshot, String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    Ok(mgr.ui_stats())
}

/// `check_update` — query the GitHub Releases API for the newest published
/// release and report whether it's newer than the running build.
///
/// The webview's CSP (`default-src 'self'`, no `connect-src`) blocks any
/// `fetch()` off-origin, so the version check can't live in the frontend —
/// it goes through Rust where `reqwest` already carries rustls.
///
/// Returns `{ current, latest, url, notes, is_newer }`:
/// - `latest`/`url`/`notes` are `null` when the API has no releases yet or the
///   call fails (offline, rate-limited) — the caller treats null latest as
///   "couldn't check", distinct from "up to date".
/// - `is_newer` is a strict semver-ish compare on the `v`-stripped tag, so a
///   same-version release is NOT flagged as an update.
#[tauri::command]
pub async fn check_update() -> Result<serde_json::Value, String> {
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
        html_url: String,
        #[serde(default)]
        body: Option<String>,
        #[serde(default)]
        draft: bool,
        #[serde(default)]
        prerelease: bool,
    }

    const RELEASES: &str = "https://api.github.com/repos/1Yie/husk/releases/latest";
    let client = reqwest::Client::builder()
        .user_agent(concat!("husk/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let current = env!("CARGO_PKG_VERSION").to_string();
    let res = client.get(RELEASES).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        // Surface the HTTP status so the UI can say "检查失败 (404)" instead
        // of silently showing "up to date".
        return Err(format!("GitHub 返回 {}", res.status()));
    }
    let rel: Release = res.json().await.map_err(|e| e.to_string())?;

    // Newest is /latest which already excludes drafts & prereleases, but
    // belt-and-suspenders in case the API shape shifts.
    if rel.draft || rel.prerelease {
        return Ok(serde_json::json!({
            "current": current, "latest": null, "url": null, "notes": null, "is_newer": false
        }));
    }

    let latest = rel.tag_name.trim_start_matches('v').to_string();
    Ok(serde_json::json!({
        "current": current,
        "latest": latest,
        "url": rel.html_url,
        "notes": rel.body,
        "is_newer": is_newer(&current, &latest),
    }))
}

/// Strict `a.b.c` numeric compare — `is_newer("0.1.9", "0.1.10")` is true.
/// Non-numeric segments (e.g. a `-rc1` suffix) make the comparison bail to
/// `false` rather than guess wrong about ordering.
fn is_newer(current: &str, latest: &str) -> bool {
    fn parts(v: &str) -> Option<Vec<u64>> {
        v.split('-')
            .next()?
            .split('.')
            .map(|p| p.parse().ok())
            .collect()
    }
    match (parts(current), parts(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

#[tauri::command]
pub fn agent_cmd(state: State<'_, KernelState>, cmd: UiCommand) -> Result<(), String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
        let (cancel, steer_tx, decision, ask, permissions, agent_mode, ui, cmd_tx) = {
        let Some(handle) = mgr.active() else {
            return Err("no active session".into());
        };
        (
            handle.cancel.clone(),
            handle.steer_tx.clone(),
            handle.decision.clone(),
            handle.ask.clone(),
            handle.permissions.clone(),
                handle.agent_mode.clone(),
            handle.ui.clone(),
            handle.cmd_tx.clone(),
        )
    };

    // `Cancel` bypasses the command pump (direct flag) so it isn't queued
    // behind a running `run_turn`.
    if matches!(cmd, UiCommand::Cancel) {
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        return Ok(());
    }
    if let UiCommand::Steer { text } = &cmd {
        steer_tx.try_send(text.clone()).map_err(|e| e.to_string())?;
        return Ok(());
    }
    if let UiCommand::ToolDecision { request_id, approved } = &cmd {
        *decision.lock().map_err(|e| e.to_string())? = Some((*request_id, *approved));
        return Ok(());
    }
    if let UiCommand::AnswerQuestion { request_id, answer } = &cmd {
        // Only a matched answer resolves the parked card — `answer()` returns
        // false for a stale/duplicate request_id, and echoing that would drop
        // a *different* live question's card.
        if ask.answer(*request_id, answer.clone()) {
            // Echo the resolution so the frontend clears `pendingQuestion`
            // from the view buffer — the answered card must not resurrect on
            // the next session switch.
            let _ = ui.send(UiEvent::QuestionAnswered {
                request_id: *request_id,
            });
        }
        return Ok(());
    }
    // Composer settings are PER-SESSION: the command goes only to the active
    // actor, which applies it to its own engine and persists it into its own
    // SessionMeta. They deliberately do NOT touch the manager-level workspace
    // default (`mgr.set_*`/`persist_prefs`) — that's what used to leak one
    // session's model/mode/level into every other session's display & spawn.
    // `SetPermissionMode`/`SetAgentMode` still write the live handle slots here
    // so a mid-turn swap applies at the next dispatch rather than after the turn.
    if let UiCommand::SetPermissionMode { mode } = &cmd {
        if let Ok(mut gate) = permissions.write() {
            *gate = agent_kernel::permissions::PermissionGate::from_mode_str(mode);
        }
    }
    if let UiCommand::SetAgentMode { mode } = &cmd {
        if let Ok(mut slot) = agent_mode.write() {
            *slot = agent_kernel::mode::AgentMode::from_str(mode);
        }
    }
    // Per-session tool whitelisting (computer-use's "本会话允许"): write
    // the live gate directly so the next dispatch sees it — forwarding
    // through `cmd_tx` would only apply it at the actor's next `handle`,
    // and a `computer` call paused in AwaitingToolConfirmation is waiting
    // on the *decision* slot, not the command pump, so the gate has to be
    // hot right now.
    if let UiCommand::ApproveSessionTool { tool_name } = &cmd {
        if let Ok(mut gate) = permissions.write() {
            gate.allow_tool_for_session(tool_name);
        }
        // Notify the UI — the computer-use overlay mounts off this event.
        let _ = ui.send(UiEvent::SessionToolWhitelisted {
            tool_name: tool_name.clone(),
            stopped: false,
        });
        return Ok(());
    }
    // The overlay's "停止" button — same direct-write path. Session-level
    // revoke so the NEXT `computer` call drops back to Ask; any running
    // call isn't retro-cancelled.
    if let UiCommand::RevokeSessionTool { tool_name } = &cmd {
        if let Ok(mut gate) = permissions.write() {
            gate.revoke_tool_for_session(tool_name);
        }
        let _ = ui.send(UiEvent::SessionToolWhitelisted {
            tool_name: tool_name.clone(),
            stopped: true,
        });
        return Ok(());
    }
    cmd_tx.try_send(cmd).map_err(|e| e.to_string())
}
