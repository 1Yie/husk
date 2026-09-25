//! `screenshot` — capture the desktop for the model's eyes.
//!
//! Read-only *and* `Observation`, so it auto-runs in `default` mode and
//! survives plan mode (looking at the screen does not change it). The PNG
//! lands in `<workspace>/.husk/attachments/` — the same staged-attachment
//! directory the composer uses, so replay and cleanup already know it — and
//! rides the wire through `ToolResult.images` when the model declares vision.
//!
//! The capture also installs the coordinate scale on the desktop backend:
//! `computer` actions take *this image's* pixel grid (see the text the tool
//! prints) and the backend maps them onto the real display.

use std::sync::Arc;

use futures::FutureExt;

use super::registry::{schema_for, ToolCtx, ToolError, ToolResult, ToolSpec};

/// No model-facing arguments: a full-screen capture is the only useful shape
/// (a region would need a screenshot first to choose it).
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ScreenshotArgs {}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "screenshot",
        schema: schema_for::<ScreenshotArgs>(
            "Capture the screen and see it. The PNG is attached to the tool \
             result, and the text reports both resolutions: `image` is the \
             grid coordinates are read off for `computer` actions, `display` \
             is the real screen size. Take a fresh screenshot after every \
             action (or a short `computer` → `wait`) before deciding the \
             next step — the screen does not stand still. Fails only when \
             no desktop backend is available.",
        ),
        readonly: true,
        class: super::registry::ToolClass::Observation,
        network: false,
        exec: Arc::new(|_args, ctx| exec(ctx).boxed()),
    }
}

async fn exec(ctx: Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    // `.husk/attachments/` — the workspace-local staging dir the composer
    // already uses for pasted images, so captures sit with their siblings.
    let dir = ctx.workspace_root.join(".husk").join("attachments");
    let dest = dir.join(format!("shot-{}.png", agent_llm::types::now_ms()));

    let meta = ctx
        .desktop
        .screenshot(&dest)
        .await
        .map_err(|e| ToolError::Failed(format!("screenshot failed: {e}")))?;

    // Workspace-relative for the text: the absolute path is long and the
    // model only needs enough to name the file.
    let shown = dest
        .strip_prefix(&*ctx.workspace_root)
        .unwrap_or(&dest)
        .display();
    let mut content = format!(
        "screenshot saved: {shown}\n\
         display: {}x{} (real screen)\n\
         image: {}x{} — use THESE coordinates for `computer` actions",
        meta.display[0], meta.display[1], meta.image[0], meta.image[1],
    );
    if ctx.desktop.id() != "x11" {
        // Should not happen (a `none` backend fails above), but a future
        // backend must announce itself rather than silently differ.
        content.push_str(&format!("\nbackend: {}", ctx.desktop.id()));
    }

    let mut res = ToolResult::text(content);
    res.ui_type = Some("screenshot");
    // The engine attaches these to the tool message; a text-only model just
    // keeps the printed path (same degradation the composer path uses).
    if let Some(img) = agent_llm::types::ImageRef::for_path(meta.path) {
        res.images.push(img);
    }
    Ok(res)
}

/// Sweep the `shot-*.png` captures this session staged under
/// `.husk/attachments/`. Called once per turn boundary — a capture's job
/// ends with the turn it fed (the model's grid and the raw file both die
/// with it), so leaving them on disk just leaks megabytes of PNG per turn.
/// Only `shot-` prefixed files are touched; user attachments that share the
/// directory are left alone. Best-effort — a missing dir or a locked file is
/// not worth failing a finished turn over.
pub fn cleanup_screenshots(workspace_root: &std::path::Path) {
    let dir = workspace_root.join(".husk").join("attachments");
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        // `shot-<ms>.png` from this tool, plus the `.shot-<ms>.tmp.png`
        // sibling a crashed capture may have left behind.
        let is_shot = (s.starts_with("shot-") && s.ends_with(".png"))
            || (s.starts_with(".shot-") && s.ends_with(".tmp.png"));
        if is_shot {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::registry::ToolRegistry;

    /// The spec's flags are the security contract: auto-run in `default`,
    /// survive plan mode, batchable.
    #[test]
    fn spec_is_a_readonly_observation() {
        let s = spec();
        assert_eq!(s.name, "screenshot");
        assert!(s.readonly);
        assert_eq!(s.class, super::super::registry::ToolClass::Observation);
        assert!(!s.network);
    }

    /// A host with no desktop backend must refuse readably, not panic — and
    /// the refusal is the probe's own reason.
    #[tokio::test]
    async fn refuses_without_a_desktop_backend() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Arc::new(ToolCtx::new(dir.path()).with_desktop(Arc::new(
            agent_computer::NoneBackend::new("$DISPLAY is not set"),
        )));
        let err = ToolRegistry::with_builtins()
            .dispatch("screenshot", serde_json::json!({}), ctx)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("screenshot failed"), "{msg}");
        assert!(msg.contains("$DISPLAY is not set"), "{msg}");
    }

    /// `cleanup_screenshots` removes this turn's captures + their temp
    /// siblings but must leave user-staged attachments alone.
    #[test]
    fn cleanup_removes_shot_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let att = dir.path().join(".husk").join("attachments");
        std::fs::create_dir_all(&att).unwrap();
        let shot = att.join("shot-1.png");
        let tmp = att.join(".shot-1.tmp.png");
        let user = att.join("pasted.png");
        let other = att.join("notes.txt");
        for p in [&shot, &tmp, &user, &other] {
            std::fs::write(p, b"x").unwrap();
        }

        cleanup_screenshots(dir.path());
        assert!(!shot.exists());
        assert!(!tmp.exists());
        assert!(user.exists());
        assert!(other.exists());

        // A missing dir is a no-op, not an error.
        cleanup_screenshots(dir.path().join("gone").as_path());
    }

    /// The built-in registry carries both computer-use tools.
    #[test]
    fn registry_carries_screenshot_and_computer() {
        let r = ToolRegistry::with_builtins();
        assert!(r.spec("screenshot").is_some());
        assert!(r.spec("computer").is_some());
        assert!(r.is_readonly("screenshot"));
        assert!(!r.is_readonly("computer"));
    }
}
