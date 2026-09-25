//! The full engine turn for a computer-use loop: the model calls
//! `screenshot`, the frame reaches the persisted tool row, and the *next*
//! request carries it as a real image part.
//!
//! This is the integration seam unit tests cannot cover: registry, permission
//! gate, engine dispatch, `ToolResult.images`, the `ChatMessage` hand-off, and
//! the adapter's wire encoding, all in one turn. `run_turn` is driven directly
//! with an injected desktop backend, so the positive path is deterministic —
//! no X server, no headless-host conditional.

use std::sync::Arc;

use agent_kernel::engine::{Engine, EngineIo};
use agent_kernel::tools::{ToolCtx, ToolRegistry};

mod common;
use agent_llm::types::{ChatMessage, StreamChunk};
use common::ScriptedProvider;

/// A desktop backend that always succeeds and writes a real 4×4 PNG.
struct FakeDesktop;

#[async_trait::async_trait]
impl agent_computer::DesktopBackend for FakeDesktop {
    fn id(&self) -> &'static str {
        "fake"
    }
    async fn screenshot(
        &self,
        dest: &std::path::Path,
    ) -> anyhow::Result<agent_computer::ScreenshotMeta> {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        image::DynamicImage::ImageRgb8(image::RgbImage::new(4, 4))
            .save_with_format(dest, image::ImageFormat::Png)?;
        Ok(agent_computer::ScreenshotMeta {
            path: dest.to_path_buf(),
            display: [1920, 1080],
            image: [4, 4],
        })
    }
    async fn click(
        &self,
        _b: agent_computer::MouseButton,
        _at: [i32; 2],
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn double_click(&self, _at: [i32; 2]) -> anyhow::Result<()> {
        Ok(())
    }
    async fn move_to(&self, _at: [i32; 2]) -> anyhow::Result<()> {
        Ok(())
    }
    async fn drag(&self, _f: [i32; 2], _t: [i32; 2]) -> anyhow::Result<()> {
        Ok(())
    }
    async fn type_text(&self, _t: &str) -> anyhow::Result<()> {
        Ok(())
    }
    async fn key(&self, _c: &str, _h: Option<u64>) -> anyhow::Result<()> {
        Ok(())
    }
    async fn scroll(
        &self,
        _d: agent_computer::ScrollDir,
        _a: i32,
        _at: Option<[i32; 2]>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn cursor_position(&self) -> anyhow::Result<[i32; 2]> {
        Ok([0, 0])
    }
    async fn launch_app(&self, _command: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

fn done() -> StreamChunk {
    StreamChunk::Done {
        prompt_tokens: None,
        completion_tokens: None,
        cached_tokens: None,
    }
}

/// A `screenshot` tool call, then a closing text answer.
fn script_screenshot_then_answer() -> Vec<StreamChunk> {
    vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("call_1".into()),
            name: Some("screenshot".into()),
            args_delta: "{}".into(),
        },
        StreamChunk::ContentDelta("I looked at the screen.".into()),
        done(),
    ]
}

/// Drive one turn against an injected desktop, with `model_input` controlling
/// the vision gate. Returns the history the turn produced.
async fn run_one_turn(
    dir: &std::path::Path,
    model_input: Vec<String>,
) -> Vec<ChatMessage> {
    let stub = Arc::new(ScriptedProvider::new().with_script(script_screenshot_then_answer()));
    stub.script_text("The desktop shows a terminal.");

    let ctx = Arc::new(
        ToolCtx::new(dir).with_desktop(Arc::new(FakeDesktop)),
    );
    let mut engine = Engine::new(
        stub,
        Arc::new(ToolRegistry::with_builtins()),
        ctx,
        "test-model",
        0.0,
    );
    engine.set_permissions(agent_kernel::permissions::PermissionGate::from_mode_str("auto"));
    engine.set_agent_mode(agent_kernel::mode::AgentMode::Build);
    engine.set_model_input(model_input);
    let (ui_tx, _ui_rx) = agent_kernel::channels::UiSink::channel();
    let (_steer_tx, steer_rx) = tokio::sync::mpsc::channel(4);
    let mut io = EngineIo {
        ui_tx,
        steer_rx,
        cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let mut history = vec![ChatMessage::system("kernel")];
    // Same tracker lifecycle `SessionActor` runs: `begin_turn` first, so any
    // write a turn produced would key to a real turn index.
    let mut hunks = agent_context::HunkTracker::new(agent_context::TrackingMode::AgentOnly);
    hunks.begin_turn();
    engine
        .run_turn(&mut io, &mut history, "take a screenshot".into(), &mut hunks)
        .await
        .expect("turn must complete");
    history
}

/// The loop, end to end: a `screenshot` call stages a PNG in
/// `.husk/attachments/` and the tool row carries the frame — which the next
/// request sends as a real image part for a vision model.
#[tokio::test]
async fn screenshot_frame_rides_the_tool_row_for_a_vision_model() {
    let dir = tempfile::tempdir().unwrap();
    let history = run_one_turn(dir.path(), vec!["text".into(), "image".into()]).await;

    let tool_row = history
        .iter()
        .find(|m| m.role == agent_llm::Role::Tool)
        .expect("a tool result row must be persisted");
    // The model-facing text carries the coordinate contract…
    let text = tool_row.content.as_deref().unwrap_or("");
    assert!(text.contains("display: 1920x1080"), "{text}");
    assert!(text.contains("image: 4x4"), "{text}");
    // …and the frame is on the row for the adapter to encode.
    assert_eq!(tool_row.images.len(), 1, "the frame must ride the tool row");
    assert!(tool_row.images[0].path.starts_with(dir.path().join(".husk/attachments")));
    assert!(
        tool_row.images[0].data_url().is_some(),
        "the staged PNG must be wire-encodable"
    );
}

/// A text-only model must not receive image parts — the engine gates on the
/// declared modalities, exactly like the composer path does. The staged file
/// still exists and its path is still in the text, so nothing is lost: the
/// model simply reads the path instead of the pixels.
#[tokio::test]
async fn text_only_model_keeps_the_tool_frame_off_the_message() {
    let dir = tempfile::tempdir().unwrap();
    let history = run_one_turn(dir.path(), vec!["text".into()]).await;

    let tool_row = history
        .iter()
        .find(|m| m.role == agent_llm::Role::Tool)
        .expect("tool row");
    assert!(
        tool_row.images.is_empty(),
        "a text-only model must not get image parts: {:?}",
        tool_row.images
    );
    // The path is still in the text, so the model can still name the file.
    let text = tool_row.content.as_deref().unwrap_or("");
    assert!(text.contains("screenshot saved"), "{text}");
    assert!(
        std::fs::read_dir(dir.path().join(".husk/attachments"))
            .map(|d| d.count())
            .unwrap_or(0)
            == 1,
        "the capture is still on disk for the user"
    );
}
