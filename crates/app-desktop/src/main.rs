//! app-desktop — Slint GUI shell.
//!
//! Stage 5 scope: mock-data UI render + kernel wiring behind a flag.
//! `--mock` (default while the bridge matures) boots with static stub models;
//! real kernel wiring lands via `bridge.rs` once visuals are confirmed.

mod bridge;
mod throttler;

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    tracing_subscriber_init();

    let app = CodexDesktop::new()?;

    if std::env::args().any(|a| a == "--live") {
        bridge::wire_kernel(&app);
    } else {
        seed_mock(&app);
    }

    app.run()
}

fn tracing_subscriber_init() {
    // keep it zero-dep: honor RUST_LOG via env_logger-style output
    if std::env::var("RUST_LOG").is_ok() {
        eprintln!("[app-desktop] logging enabled");
    }
}

/// Static stub models — Phase 1 of the contract's implementation phasing:
/// visuals verified before the kernel is attached.
fn seed_mock(app: &CodexDesktop) {
    use slint::VecModel;

    let bridge = app.global::<Bridge>();

    bridge.set_stats(SessionStats {
        agent_state: slint::SharedString::from("Idle"),
        permission_mode: slint::SharedString::from("default"),
        active_provider: slint::SharedString::from("grok"),
        active_model: slint::SharedString::from("grok-4"),
        tokens_used: 12_400,
        context_window: 256_000,
        files_changed: 2,
        degraded: false,
    });

    let messages: Vec<SessionMessageData> = vec![
        SessionMessageData {
            id: 0,
            role: slint::SharedString::from("user"),
            text: slint::SharedString::from("fix the borrow error in engine.rs"),
            reasoning: slint::SharedString::new(),
            has_diff: false,
            streaming: false,
        },
        SessionMessageData {
            id: 1,
            role: slint::SharedString::from("agent"),
            text: slint::SharedString::from("Reading the engine module to locate the borrow conflict."),
            reasoning: slint::SharedString::from("scanning src/engine.rs…"),
            has_diff: true,
            streaming: false,
        },
    ];
    bridge.set_messages(std::rc::Rc::new(VecModel::from(messages)).into());

    let steps: Vec<ActionStepData> = vec![
        ActionStepData {
            id: 0,
            name: slint::SharedString::from("smart_read"),
            state: slint::SharedString::from("success"),
            detail: slint::SharedString::from("src/engine.rs · 48 lines"),
            expandable: true,
        },
        ActionStepData {
            id: 1,
            name: slint::SharedString::from("fuzzy_patch"),
            state: slint::SharedString::from("awaiting_confirm"),
            detail: slint::SharedString::from("src/engine.rs · +14 −3"),
            expandable: true,
        },
    ];
    bridge.set_active_steps(std::rc::Rc::new(VecModel::from(steps)).into());

    bridge.set_pending(PendingApprovalData {
        step_id: 1,
        tool_name: slint::SharedString::from("fuzzy_patch"),
        title: slint::SharedString::from("Edit crates/agent-kernel/src/engine.rs"),
        command: slint::SharedString::new(),
        risk: slint::SharedString::from("normal"),
        audit_reason: slint::SharedString::new(),
        diff_text: slint::SharedString::new(),
        fuzzy: false,
    });

    let diff: Vec<DiffLineData> = vec![
        DiffLineData { line_type: "context".into(), content: "    pub async fn run_turn(".into(), old_lineno: 40, new_lineno: 40 },
        DiffLineData { line_type: "delete".into(), content: "        self.state = State::Idle;".into(), old_lineno: 41, new_lineno: -1 },
        DiffLineData { line_type: "add".into(), content: "        self.state = State::Running;".into(), old_lineno: -1, new_lineno: 41 },
        DiffLineData { line_type: "add".into(), content: "        self.notify();".into(), old_lineno: -1, new_lineno: 42 },
        DiffLineData { line_type: "context".into(), content: "    }".into(), old_lineno: 42, new_lineno: 43 },
    ];
    bridge.set_pending_diff(std::rc::Rc::new(VecModel::from(diff)).into());

    bridge.set_changed_files(std::rc::Rc::new(VecModel::from(vec![
        slint::SharedString::from("crates/agent-kernel/src/engine.rs"),
        slint::SharedString::from("crates/agent-kernel/src/session.rs"),
    ])).into());

    bridge.set_model_options(std::rc::Rc::new(VecModel::from(vec![
        ModelOption {
            provider_id: "grok".into(),
            label: "grok-4 (xAI)".into(),
            model: "grok-4".into(),
            available: true,
        },
        ModelOption {
            provider_id: "ollama".into(),
            label: "qwen3 (local)".into(),
            model: "qwen3".into(),
            available: true,
        },
    ])).into());
}
