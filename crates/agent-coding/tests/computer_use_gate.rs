//! Engine-level guarantees of the `computer` approval path, driven through a
//! real `run_turn`: what the approval card shows, that a session grant does
//! not cover `launch`, that a mode switch keeps the grant, and that one turn
//! cannot drive the desktop without bound.

use std::sync::{Arc, Mutex};

use agent_ipc::UiEvent;
use agent_kernel::engine::{Engine, EngineIo};
use agent_kernel::permissions::{PermissionGate, PermissionMode};
use agent_kernel::tools::{ToolCtx, ToolRegistry};
use agent_llm::types::{ChatMessage, StreamChunk};

use agent_testkit::ScriptedProvider;

/// Records every input call so a test can say what actually reached the
/// desktop, and what did not.
#[derive(Default)]
struct Recorder {
    /// Shared so the approval callback can write into the same log.
    calls: Arc<Mutex<Vec<String>>>,
    refuse_prepare: bool,
}

impl Recorder {
    fn shared_log(&self) -> Arc<Mutex<Vec<String>>> {
        self.calls.clone()
    }
}

#[async_trait::async_trait]
impl agent_computer::DesktopBackend for Recorder {
    fn id(&self) -> &'static str {
        "recorder"
    }
    async fn screenshot(
        &self,
        dest: &std::path::Path,
    ) -> anyhow::Result<agent_computer::ScreenshotMeta> {
        Ok(agent_computer::ScreenshotMeta {
            path: dest.to_path_buf(),
            display: [10, 10],
            image: [10, 10],
        })
    }
    async fn click(&self, _b: agent_computer::MouseButton, at: [i32; 2]) -> anyhow::Result<()> {
        self.calls.lock().unwrap().push(format!("click {at:?}"));
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
    async fn type_text(&self, t: &str) -> anyhow::Result<()> {
        self.calls.lock().unwrap().push(format!("type {t}"));
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
    async fn launch_app(&self, command: &str) -> anyhow::Result<()> {
        self.calls.lock().unwrap().push(format!("launch {command}"));
        Ok(())
    }
    async fn prepare(&self) -> anyhow::Result<()> {
        self.calls.lock().unwrap().push("prepare".into());
        if self.refuse_prepare {
            anyhow::bail!("input access was refused");
        }
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

/// One assistant round that issues the given `computer` calls.
fn round(calls: &[serde_json::Value]) -> Vec<StreamChunk> {
    let mut v: Vec<StreamChunk> = calls
        .iter()
        .enumerate()
        .map(|(i, a)| StreamChunk::ToolCallDelta {
            slot: i,
            id: Some(format!("call_{i}")),
            name: Some("computer".into()),
            args_delta: a.to_string(),
        })
        .collect();
    v.push(done());
    v
}

struct Rig {
    engine: Engine,
    desk: Arc<Recorder>,
    dir: tempfile::TempDir,
}

impl Rig {
    fn desk_log(&self) -> Arc<Mutex<Vec<String>>> {
        self.desk.shared_log()
    }
}

fn rig(mode: &str, grant: &[&str], scripts: Vec<Vec<StreamChunk>>) -> Rig {
    rig_with(mode, grant, scripts, false)
}

fn rig_with(
    mode: &str,
    grant: &[&str],
    scripts: Vec<Vec<StreamChunk>>,
    refuse_prepare: bool,
) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let desk = Arc::new(Recorder {
        refuse_prepare,
        ..Default::default()
    });
    let stub = Arc::new(ScriptedProvider::new());
    for s in scripts {
        stub.push_script(s);
    }
    stub.script_text("done");
    let ctx = Arc::new(ToolCtx::new(dir.path()).with_desktop(desk.clone()));
    let mut engine = Engine::new(
        stub,
        Arc::new(ToolRegistry::with_builtins()),
        ctx.clone(),
        "test-model",
        0.0,
    );
    // The turn contracts (the `computer` cap among them) are the coding
    // policy now — a bare `Engine::new` installs no product rules.
    engine.set_policy(Box::new(agent_coding::policy::CodingPolicy::from_ctx(&ctx)));
    let mut gate = PermissionGate::from_mode_str(mode);
    for g in grant {
        gate.allow_tool_for_session(g);
    }
    engine.set_permissions(gate);
    engine.set_agent_mode("build");
    Rig { engine, desk, dir }
}

/// Run one turn; `answer` is called for every approval card with
/// `(tool, diff)` and returns the verdict. Returns `(history, cards)`.
async fn run(
    r: &mut Rig,
    answer: impl Fn(&str, &str) -> bool + Send + Sync + 'static,
) -> (Vec<ChatMessage>, Vec<(String, String)>) {
    let (ui_tx, mut ui_rx) = agent_kernel::channels::UiSink::channel();
    let (_steer_tx, steer_rx) = tokio::sync::mpsc::channel(4);
    let mut io = EngineIo {
        ui_tx,
        steer_rx,
        cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let slot = r.engine.decision_slot();
    let cards = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let cards2 = cards.clone();
    // Cards and input share one log, so a test can assert their order.
    let card_log = Some(r.desk_log());
    let pump = tokio::spawn(async move {
        while let Some(ev) = ui_rx.recv().await {
            if let UiEvent::ApprovalRequested {
                request_id,
                tool_name,
                diff,
                ..
            } = ev
            {
                if let Some(log) = &card_log {
                    log.lock().unwrap().push(format!("card {tool_name}"));
                }
                let ok = answer(&tool_name, &diff);
                cards2.lock().unwrap().push((tool_name, diff));
                *slot.lock().unwrap() = Some((request_id, ok));
            }
        }
    });
    let mut history = vec![ChatMessage::system("kernel")];
    r.engine
        .run_turn(&mut io, &mut history, "go".into())
        .await
        .expect("turn completes");
    // The engine/ctx may still hold sink clones, so the channel never closes
    // by itself: give the pump a moment to drain what is queued, then stop it.
    drop(io);
    let _ = tokio::time::timeout(std::time::Duration::from_millis(200), pump).await;
    let cards = cards.lock().unwrap().clone();
    (history, cards)
}

/// What actually reached the desktop, in order: the control events
/// (`prepare`, `card …`) share the log but are not input.
fn actions(r: &Rig) -> Vec<String> {
    r.desk
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|e| *e != "prepare" && !e.starts_with("card "))
        .cloned()
        .collect()
}

fn tool_results(h: &[ChatMessage]) -> Vec<String> {
    h.iter()
        .filter(|m| m.role == agent_llm::Role::Tool)
        .map(|m| m.content.clone().unwrap_or_default())
        .collect()
}

/// The card must show what will be typed/launched, not just the action name —
/// otherwise "allow for this session" is a blind signature.
#[tokio::test]
async fn approval_card_shows_the_text_and_command() {
    let mut r = rig(
        "default",
        &[],
        vec![round(&[
            serde_json::json!({"action": "type", "text": "echo pwned\n"}),
            serde_json::json!({"action": "launch", "text": "kwrite /etc/hosts"}),
        ])],
    );
    let (_h, cards) = run(&mut r, |_, _| false).await;
    let diffs: Vec<&str> = cards.iter().map(|c| c.1.as_str()).collect();
    assert_eq!(
        diffs,
        ["type \"echo pwned\\n\"", "launch \"kwrite /etc/hosts\""]
    );
    assert!(actions(&r).is_empty(), "denied: nothing ran");
}

/// A session grant lets clicks and typing through without a card, but
/// `launch` still asks — and runs only when approved.
#[tokio::test]
async fn session_grant_does_not_cover_launch() {
    let calls = [
        serde_json::json!({"action": "left_click", "coordinate": [1, 2]}),
        serde_json::json!({"action": "launch", "text": "kwrite a.txt"}),
        serde_json::json!({"action": "type", "text": "hi"}),
    ];

    // Denied card: click + type run, launch does not.
    let mut r = rig("default", &["computer"], vec![round(&calls)]);
    let (_h, cards) = run(&mut r, |_, _| false).await;
    assert_eq!(cards.len(), 1, "only launch asks: {cards:?}");
    assert_eq!(cards[0].1, "launch \"kwrite a.txt\"");
    assert_eq!(
        actions(&r),
        ["click [1, 2]".to_string(), "type hi".to_string()]
    );

    // Approved card: launch runs too, in order.
    let mut r = rig("default", &["computer"], vec![round(&calls)]);
    let (_h, cards) = run(&mut r, |_, _| true).await;
    assert_eq!(cards.len(), 1);
    assert_eq!(
        actions(&r),
        [
            "click [1, 2]".to_string(),
            "launch kwrite a.txt".to_string(),
            "type hi".to_string()
        ]
    );
}

/// An explicit `computer:launch` allow rule pre-approves it.
#[tokio::test]
async fn explicit_launch_rule_skips_the_card() {
    let mut r = rig(
        "default",
        &["computer", "computer:launch"],
        vec![round(&[serde_json::json!({"action": "launch", "text": "kwrite"})])],
    );
    let (_h, cards) = run(&mut r, |_, _| false).await;
    assert!(cards.is_empty());
    assert_eq!(actions(&r), ["launch kwrite".to_string()]);
}

/// `dontAsk` + grant: nobody can answer, so launch is refused with a reason.
#[tokio::test]
async fn launch_is_denied_under_dont_ask() {
    let mut r = rig(
        "dontAsk",
        &["computer"],
        vec![round(&[serde_json::json!({"action": "launch", "text": "kwrite"})])],
    );
    let (h, cards) = run(&mut r, |_, _| true).await;
    assert!(cards.is_empty());
    assert!(actions(&r).is_empty());
    assert!(
        tool_results(&h)[0].contains("needs its own approval"),
        "{:?}",
        tool_results(&h)
    );
}

/// With a grant in place a runaway loop is still bounded per turn: the model
/// is told to stop, and the desktop sees no more than the cap.
#[tokio::test]
async fn one_turn_cannot_drive_the_desktop_without_bound() {
    const CAP: usize = 60;
    // Distinct coordinates: the engine's doom-loop guard drops a round of
    // byte-identical calls before the gate ever sees them.
    let batch: Vec<serde_json::Value> = (0..CAP + 5)
        .map(|i| serde_json::json!({"action": "left_click", "coordinate": [i, i]}))
        .collect();
    let mut r = rig("default", &["computer"], vec![round(&batch)]);
    let (h, _cards) = run(&mut r, |_, _| true).await;

    let results = tool_results(&h);
    assert_eq!(actions(&r).len(), CAP);
    assert_eq!(results.len(), CAP + 5, "every call still gets a result row");
    assert!(
        results.last().unwrap().contains("action limit"),
        "{}",
        results.last().unwrap()
    );
}

/// Switching permission mode in place keeps the session grant (the gate used
/// to be rebuilt, silently dropping it while the overlay stayed up).
#[tokio::test]
async fn mode_switch_keeps_the_session_grant() {
    let mut r = rig(
        "default",
        &["computer"],
        vec![round(&[serde_json::json!({"action": "left_click", "coordinate": [3, 4]})])],
    );
    {
        let slot = r.engine.permissions_writer();
        slot.write()
            .unwrap()
            .set_mode(PermissionMode::from_str("acceptEdits"));
    }
    let (_h, cards) = run(&mut r, |_, _| false).await;
    assert!(cards.is_empty(), "grant survived the switch: {cards:?}");
    assert_eq!(actions(&r), ["click [3, 4]".to_string()]);
    let _ = &r.dir;
}

/// The desktop grant is requested BEFORE the in-app approval card, so the
/// system prompt comes first and the card is never followed by a surprise.
#[tokio::test]
async fn desktop_grant_is_requested_before_the_approval_card() {
    let mut r = rig(
        "default",
        &[],
        vec![round(&[serde_json::json!({"action": "left_click", "coordinate": [1, 2]})])],
    );
    let log = r.desk_log();
    let (_h, cards) = run(&mut r, |_, _| true).await;
    assert_eq!(cards.len(), 1);
    let order = log.lock().unwrap().clone();
    let prepare = order.iter().position(|e| e == "prepare").expect("prepare ran");
    let card = order
        .iter()
        .position(|e| e.starts_with("card "))
        .expect("card shown");
    assert!(prepare < card, "prepare must precede the card: {order:?}");
    let clicks = r
        .desk
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e.starts_with("click"))
        .count();
    assert_eq!(clicks, 1, "the approved click ran once");
}

/// If the grant is refused, the approval card is never shown and nothing runs.
#[tokio::test]
async fn refused_grant_skips_the_approval_card() {
    let mut r = rig_with(
        "default",
        &[],
        vec![round(&[serde_json::json!({"action": "left_click", "coordinate": [1, 2]})])],
        true,
    );
    let (h, cards) = run(&mut r, |_, _| true).await;
    assert!(cards.is_empty(), "no card for an action that cannot run: {cards:?}");
    assert!(actions(&r).is_empty(), "refused grant: no input ran");
    assert!(
        tool_results(&h)[0].contains("computer control unavailable"),
        "{:?}",
        tool_results(&h)
    );
}

/// The desktop cap counts `computer` actions only. A turn that runs more
/// non-desktop calls than the cap must still leave the full desktop budget for
/// the computer calls that follow it.
#[tokio::test]
async fn non_desktop_calls_do_not_spend_the_desktop_budget() {
    const CAP: usize = 60;
    // Cap+10 bash calls (each distinct, so the doom-loop guard keeps them),
    // then three desktop actions in a later round.
    let bash_calls: Vec<serde_json::Value> = (0..CAP + 10)
        .map(|i| serde_json::json!({"command": format!("echo {i}")}))
        .collect();
    let mut bash_round: Vec<StreamChunk> = bash_calls
        .iter()
        .enumerate()
        .map(|(i, a)| StreamChunk::ToolCallDelta {
            slot: i,
            id: Some(format!("bash_{i}")),
            name: Some("bash".into()),
            args_delta: a.to_string(),
        })
        .collect();
    bash_round.push(done());
    let desk_round = round(&[
        serde_json::json!({"action": "left_click", "coordinate": [1, 1]}),
        serde_json::json!({"action": "left_click", "coordinate": [2, 2]}),
        serde_json::json!({"action": "left_click", "coordinate": [3, 3]}),
    ]);
    let mut r = rig("default", &["computer", "bash"], vec![bash_round, desk_round]);
    let (_h, _cards) = run(&mut r, |_, _| true).await;

    assert_eq!(
        actions(&r).len(),
        3,
        "bash calls must not consume the desktop budget: {:?}",
        actions(&r)
    );
}
