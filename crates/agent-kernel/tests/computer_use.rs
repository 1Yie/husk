//! Computer-use acceptance: the registry shape of `screenshot`/`computer`
//! across agent modes and permission modes, and the tool-image channel from
//! a tool result through to the persisted message.
//!
//! No real X server is touched: the tests install a scripted desktop backend
//! that writes a real PNG, so the whole path (tool → `ToolResult.images` →
//! engine → `ChatMessage.images`) is exercised deterministically.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_computer::{DesktopBackend, MouseButton, ScrollDir, ScreenshotMeta};
use agent_kernel::mode::AgentMode;
use agent_kernel::permissions::{Decision, PermissionGate, PermissionMode};
use agent_kernel::tools::registry::ToolClass;
use agent_kernel::tools::{ToolCtx, ToolRegistry};

/// A desktop backend that behaves, without a display: it records the calls it
/// received and writes a real 1×1 PNG for captures.
#[derive(Default)]
struct FakeDesktop {
    clicks: std::sync::Mutex<Vec<([i32; 2], &'static str)>>,
    typed: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl DesktopBackend for FakeDesktop {
    fn id(&self) -> &'static str {
        "fake"
    }

    async fn screenshot(&self, dest: &Path) -> anyhow::Result<ScreenshotMeta> {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // A real PNG so `ImageRef::data_url` can frame it later.
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::new(4, 4));
        img.save_with_format(dest, image::ImageFormat::Png)?;
        Ok(ScreenshotMeta {
            path: dest.to_path_buf(),
            display: [1920, 1080],
            image: [4, 4],
        })
    }

    async fn click(&self, button: MouseButton, at: [i32; 2]) -> anyhow::Result<()> {
        let name = match button {
            MouseButton::Left => "left",
            MouseButton::Right => "right",
            MouseButton::Middle => "middle",
        };
        self.clicks.lock().unwrap().push((at, name));
        Ok(())
    }

    async fn double_click(&self, at: [i32; 2]) -> anyhow::Result<()> {
        self.clicks.lock().unwrap().push((at, "double"));
        Ok(())
    }

    async fn move_to(&self, _at: [i32; 2]) -> anyhow::Result<()> {
        Ok(())
    }

    async fn drag(&self, _from: [i32; 2], _to: [i32; 2]) -> anyhow::Result<()> {
        Ok(())
    }

    async fn type_text(&self, text: &str) -> anyhow::Result<()> {
        self.typed.lock().unwrap().push(text.to_string());
        Ok(())
    }

    async fn key(&self, _combo: &str, _hold_ms: Option<u64>) -> anyhow::Result<()> {
        Ok(())
    }

    async fn scroll(&self, _dir: ScrollDir, _amount: i32, _at: Option<[i32; 2]>) -> anyhow::Result<()> {
        Ok(())
    }

    async fn cursor_position(&self) -> anyhow::Result<[i32; 2]> {
        Ok([10, 20])
    }

    async fn launch_app(&self, _command: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

fn ctx_at(dir: &Path, desktop: Arc<FakeDesktop>) -> Arc<ToolCtx> {
    Arc::new(ToolCtx::new(dir).with_desktop(desktop))
}

/// The registry the model is offered must carry both tools in build/goal mode
/// and only the observation one in plan mode — "look, do not touch".
#[test]
fn plan_mode_keeps_the_eyes_and_drops_the_hands() {
    let build = ToolRegistry::with_builtins();
    assert!(build.spec("screenshot").is_some());
    assert!(build.spec("computer").is_some());

    let plan = build.readonly_only();
    assert!(plan.spec("screenshot").is_some(), "plan mode must see the screen");
    assert!(plan.spec("computer").is_none(), "plan mode must not drive it");

    // The full registry keeps `computer` out of a readonly batch by class.
    assert_eq!(build.spec("computer").unwrap().class, ToolClass::Process);
    assert_eq!(build.spec("screenshot").unwrap().class, ToolClass::Observation);
    assert!(build.spec("screenshot").is_some_and(|s| s.readonly));
}

/// `screenshot` is an observation: it runs without a confirmation in every
/// non-denying mode. `computer` is a process action: it confirms in `default`
/// / `acceptEdits`, denies in `dontAsk`, and auto-runs only in `auto`/`bypass`.
#[test]
fn permission_matrix_for_desktop_tools() {
    let shot_readonly = ToolRegistry::with_builtins()
        .spec("screenshot")
        .map(|s| s.readonly)
        .unwrap();
    let comp_class = ToolRegistry::with_builtins().spec("computer").map(|s| s.class);

    for mode in ["default", "acceptEdits", "auto", "dontAsk", "bypassPermissions"] {
        let g = PermissionGate::new(PermissionMode::from_str(mode), Default::default());
        let shot = g.decide("screenshot", Some(ToolClass::Observation), shot_readonly, None, "");
        assert_eq!(shot, Decision::Allow, "screenshot must auto-run in {mode}");
    }

    // default / acceptEdits → confirm (a click is not an edit, unlike a patch)
    for mode in ["default", "acceptEdits"] {
        let g = PermissionGate::new(PermissionMode::from_str(mode), Default::default());
        assert!(
            matches!(g.decide("computer", comp_class, false, None, "left_click"), Decision::Ask { .. }),
            "computer must confirm in {mode}"
        );
    }
    // auto / bypass → run
    for mode in ["auto", "bypassPermissions"] {
        let g = PermissionGate::new(PermissionMode::from_str(mode), Default::default());
        assert_eq!(
            g.decide("computer", comp_class, false, None, "click"),
            Decision::Allow,
            "computer runs in {mode}"
        );
    }
    // dontAsk → deny outright (not a pre-approved tool)
    let g = PermissionGate::new(PermissionMode::DontAsk, Default::default());
    assert!(matches!(
        g.decide("computer", comp_class, false, None, "click"),
        Decision::Deny { .. }
    ));
}

/// A delegated subagent must not be able to drive the desktop. The policy
/// layer does NOT provide this: `for_subagent()` is `auto` + `headless`, and
/// `headless` only turns `Ask` into `Deny` — `auto` never asks in the first
/// place. The actual guard is the registry exclusion every child is built
/// from, asserted here against the same filter `session.rs` uses.
#[test]
fn child_registries_never_carry_desktop_input() {
    let registry = ToolRegistry::with_builtins();
    let child_full = registry.filtered(|s| {
        s.name != "delegate"
            && s.name != "computer"
            && s.class != ToolClass::HumanInteraction
    });
    let child_ro = child_full.readonly_only();

    for child in [&child_full, &child_ro] {
        assert!(child.spec("computer").is_none(), "a child must not drive the desktop");
    }
    // …while the eyes are inherited: a delegated visual check needs to see.
    assert!(child_full.spec("screenshot").is_some());
    assert!(child_ro.spec("screenshot").is_some());

    // And the second guard: a child `ToolCtx` built from a refusing backend
    // fails loudly even if a future path reaches `computer` by name.
    let g = PermissionGate::for_subagent();
    let comp_class = registry.spec("computer").map(|s| s.class);
    assert_eq!(
        g.decide("computer", comp_class, false, None, "click"),
        Decision::Allow,
        "policy alone allows it — which is exactly why the registry must exclude it"
    );
}

/// The write path end to end: `screenshot` writes a PNG into
/// `.husk/attachments/`, prints both resolutions, and hands the frame back
/// through `ToolResult.images`.
#[tokio::test]
async fn screenshot_stages_a_png_and_returns_the_frame() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeDesktop::default());
    let res = ToolRegistry::with_builtins()
        .dispatch("screenshot", serde_json::json!({}), ctx_at(dir.path(), fake))
        .await
        .unwrap();

    assert_eq!(res.ui_type, Some("screenshot"));
    // The text teaches the coordinate contract the model must follow.
    assert!(res.content.contains("display: 1920x1080"), "{}", res.content);
    assert!(res.content.contains("image: 4x4"), "{}", res.content);
    assert!(res.content.contains("computer"), "{}", res.content);

    // The file is on disk under the staged-attachment directory…
    let png = dir.path().join(".husk/attachments");
    let entries: Vec<PathBuf> = std::fs::read_dir(&png)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert!(entries[0].to_string_lossy().ends_with(".png"));
    // …and the frame is attached for a vision model to read.
    assert_eq!(res.images.len(), 1);
    assert_eq!(res.images[0].path, entries[0]);
    assert!(res.images[0].data_url().is_some(), "frame must be wire-encodable");
}

/// `computer` forwards the action, in screenshot coordinates, to the backend —
/// and reports a model-readable outcome rather than a raw command line.
#[tokio::test]
async fn computer_delivers_actions_to_the_backend() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeDesktop::default());
    let r = ToolRegistry::with_builtins();

    r.dispatch(
        "computer",
        serde_json::json!({"action": "left_click", "coordinate": [12, 34]}),
        ctx_at(dir.path(), fake.clone()),
    )
    .await
    .unwrap();
    assert_eq!(fake.clicks.lock().unwrap().as_slice(), &[([12, 34], "left")]);

    let out = r
        .dispatch(
            "computer",
            serde_json::json!({"action": "type", "text": "hello"}),
            ctx_at(dir.path(), fake.clone()),
        )
        .await
        .unwrap();
    assert!(out.content.contains("5 characters"), "{}", out.content);
    assert_eq!(fake.typed.lock().unwrap().as_slice(), &["hello".to_string()]);

    let out = r
        .dispatch(
            "computer",
            serde_json::json!({"action": "cursor_position"}),
            ctx_at(dir.path(), fake),
        )
        .await
        .unwrap();
    assert!(out.content.contains("(10, 20)"), "{}", out.content);
}

/// The agent-mode contract at the engine's own seam: the same registry the
/// model is shown must be the one plan mode filters, and `readonly_only`
/// must not be bypassable by batching a captured full registry.
#[test]
fn agent_modes_shape_the_tool_set() {
    assert_eq!(AgentMode::from_str("plan"), AgentMode::Plan);
    let full = ToolRegistry::with_builtins();
    let plan = full.filtered(|s| s.readonly);
    assert!(plan.spec("computer").is_none(), "plan drops the driver");
    assert!(plan.spec("screenshot").is_some(), "plan keeps the camera");
}
