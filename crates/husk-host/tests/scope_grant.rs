//! The full loop for a directory outside the workspace: a tool refuses, the
//! host turns that refusal into a grant question, and the SAME call runs again
//! once the user accepts. This is the path the model cannot see — it only
//! observes "the call failed, then the same call worked".

use std::sync::Arc;

use agent_capabilities::{LinkedRoots, WorkspaceCtx};
use agent_ipc::UiEvent;
use agent_kernel::engine::{Engine, EngineIo};
use agent_kernel::tools::{ToolCtx, ToolRegistry};
use agent_llm::types::{ChatMessage, StreamChunk};
use agent_store::SessionStore;
use agent_testkit::ScriptedProvider;

struct Fixture {
    _dir: tempfile::TempDir,
    ws: std::path::PathBuf,
    other: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let [ws, other] = ["app", "kernel"].map(|n| base.join(n));
    for d in [&ws, &other] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(other.join("lib.rs"), "pub fn linked() {}\n").unwrap();
    Fixture {
        _dir: dir,
        ws,
        other,
    }
}

/// The app-wide paths for a fixture workspace — what every hub in these tests
/// validates against.
fn app_paths(f: &Fixture) -> agent_paths::AppPaths {
    agent_paths::AppPaths::new(f.ws.join(".config"), f.ws.join(".data"), ".husk")
}

/// The context a real session builds: the grant callback hands the question to
/// the workspace's link hub — exactly what `SessionActor` installs, so a broken
/// retry shows up here rather than in the app.
fn session_ctx(f: &Fixture, _store: Arc<SessionStore>) -> (Arc<ToolCtx>, Arc<LinkedRoots>) {
    let roots = Arc::new(LinkedRoots::default());
    let hub = husk_host::links::hub_for(&f.ws, &app_paths(f));
    roots.set(hub.effective(1));
    // The actor registers itself, so a grant refreshed by the hub lands in
    // this handle — without it the handle would only ever hold the list it
    // started with.
    let (notice_tx, notice_rx) = tokio::sync::mpsc::channel(8);
    // The receiver belongs to the actor, which these tests do not run: leak it
    // so `try_send` from a refresh has somewhere to land instead of failing.
    std::mem::forget(notice_rx);
    hub.register(1, &roots, &notice_tx);
    let mut ctx = ToolCtx::new().with_workspace(&f.ws);
    ctx.install_linked_roots(roots.clone());
    ctx.install_scope_grant(Arc::new(move |action: &str, path: &str, access: &str| {
        let Some(layer) = husk_host::links::Layer::from_action(action) else {
            return Err(format!("`{action}` is not a grant"));
        };
        hub.grant(1, path, access, layer).map(|_| ())
    }));
    (Arc::new(ctx), roots)
}

/// Drive one turn whose single tool call is `smart_read` on an out-of-scope
/// path, answering the card with `verdict`. Returns (cards seen, the actions
/// offered on the first card, the finished history).
async fn run_once(
    f: &Fixture,
    approve: Option<&str>,
) -> (usize, Vec<String>, Vec<ChatMessage>) {
    let store = Arc::new(SessionStore::open(&f.ws).unwrap());
    let (ctx, _roots) = session_ctx(f, store);
    let provider = Arc::new(ScriptedProvider::new());
    provider.push_script(vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("read-1".into()),
            name: Some("smart_read".into()),
            args_delta: serde_json::json!({
                "path": f.other.join("lib.rs").to_string_lossy(),
            })
            .to_string(),
        },
        StreamChunk::Done {
            prompt_tokens: None,
            completion_tokens: None,
            cached_tokens: None,
        },
    ]);
    provider.script_text("done");
    let registry: Arc<ToolRegistry> = Arc::new(husk_host::session::base_registries().0);
    let mut engine = Engine::new(provider, registry, ctx, "test", 0.0);
    let slot = engine.decision_slot();
    let (ui_tx, mut ui_rx) = agent_kernel::channels::UiSink::channel();
    let (_steer_tx, steer_rx) = tokio::sync::mpsc::channel(4);
    let mut io = EngineIo {
        ui_tx,
        steer_rx,
        notice_rx: tokio::sync::mpsc::channel(4).1,
        cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let mut history = vec![ChatMessage::system("test")];
    let mut cards = 0usize;
    let mut offered: Vec<String> = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let turn = engine.run_turn(&mut io, &mut history, "read it".into());
        tokio::pin!(turn);
        loop {
            tokio::select! {
                result = &mut turn => {
                    result.unwrap();
                    break;
                }
                event = ui_rx.recv() => {
                    if let Some(UiEvent::ApprovalRequested { request_id, actions, .. }) = event {
                        cards += 1;
                        if offered.is_empty() {
                            offered = actions.iter().map(|a| a.id.clone()).collect();
                        }
                        *slot.lock().unwrap() = Some((
                            request_id,
                            agent_kernel::tools::registry::ToolVerdict {
                                approved: approve.is_some(),
                                action: approve.map(str::to_string),
                            },
                        ));
                    }
                }
            }
        }
    })
    .await
    .expect("turn must finish");
    (cards, offered, history)
}

/// Accepting a grant makes the refused call succeed on the retry — the model
/// never has to try again.
#[tokio::test]
async fn an_accepted_grant_retries_the_same_call() {
    let f = fixture();
    let (cards, offered, history) = run_once(&f, Some("link:session")).await;
    assert_eq!(cards, 1, "one grant question");
    assert!(
        offered.contains(&"link:session".to_string())
            && offered.contains(&"link:workspace".to_string()),
        "both scopes are offered: {offered:?}"
    );
    let results: String = history
        .iter()
        .filter(|m| m.role == agent_llm::Role::Tool)
        .filter_map(|m| m.content.clone())
        .collect();
    assert!(
        results.contains("linked"),
        "the retried read returns the file: {results}"
    );
    // The grant landed in the session layer, which is what the menu lists.
    let store = SessionStore::open(&f.ws).unwrap();
    let links = store.load_session_links(1);
    assert_eq!(links.len(), 1);
    assert_eq!(
        links[0].path.canonicalize().unwrap(),
        f.other.canonicalize().unwrap()
    );
}

/// Refusing it leaves the call failed and grants nothing — no silent widening.
#[tokio::test]
async fn a_declined_grant_grants_nothing_and_the_call_stays_failed() {
    let f = fixture();
    let (cards, _offered, history) = run_once(&f, None).await;
    assert_eq!(cards, 1);
    let results: String = history
        .iter()
        .filter(|m| m.role == agent_llm::Role::Tool)
        .filter_map(|m| m.content.clone())
        .collect();
    assert!(
        !results.contains("pub fn linked"),
        "nothing may be read: {results}"
    );
    assert!(
        results.contains("escapes workspace") || results.contains("did not grant"),
        "the failure is reported: {results}"
    );
    let store = SessionStore::open(&f.ws).unwrap();
    assert!(store.load_session_links(1).is_empty());
    assert!(store.load_prefs().linked_roots.is_empty());
}

/// A read-write grant is what the file tools need to EDIT there — the same
/// boundary the gate holds for a workspace path.
#[tokio::test]
async fn a_granted_root_takes_a_patch_through_the_same_context() {
    let f = fixture();
    let (ctx, _roots) = session_ctx(&f, Arc::new(SessionStore::open(&f.ws).unwrap()));
    // Before the grant the patch refuses…
    let registry = husk_host::session::base_registries().0;
    let target = f.other.join("lib.rs");
    let patch = serde_json::json!({
        "path": target.to_string_lossy(),
        "search": "pub fn linked() {}",
        "replace": "pub fn linked() -> u8 { 1 }",
    });
    assert!(registry
        .dispatch("fuzzy_patch", patch.clone(), ctx.clone())
        .await
        .is_err());
    // …after it, the same call stages a commit.
    ctx.grant_scope("link:session", &target.to_string_lossy(), "readwrite")
        .expect("host installed")
        .expect("granted");
    let staged = registry
        .dispatch("fuzzy_patch", patch, ctx.clone())
        .await
        .expect("a granted root is in scope");
    staged
        .commit
        .expect("staged")
        .apply
        .await
        .expect("applied");
    assert!(std::fs::read_to_string(&target)
        .unwrap()
        .contains("-> u8"));
}

/// A refusal raised by a READ asks for read access. The card a person answers
/// was "show me this file"; granting writes to a whole external repository
/// because of it would be a permission they never saw.
#[tokio::test]
async fn a_read_refusal_grants_read_only() {
    let f = fixture();
    let (cards, _offered, _history) = run_once(&f, Some("link:session")).await;
    assert_eq!(cards, 1);
    let store = SessionStore::open(&f.ws).unwrap();
    let links = store.load_session_links(1);
    assert_eq!(links.len(), 1, "the grant landed: {links:?}");
    assert_eq!(links[0].access, "read", "a read grants read: {links:?}");

    // …and the file tools hold that line: reading works, writing does not.
    let (ctx, _roots) = session_ctx(&f, Arc::new(SessionStore::open(&f.ws).unwrap()));
    ctx.grant_scope("link:session", &f.other.join("lib.rs").to_string_lossy(), "read")
        .expect("host installed")
        .expect("granted");
    let registry = husk_host::session::base_registries().0;
    let target = f.other.join("lib.rs");
    registry
        .dispatch(
            "smart_read",
            serde_json::json!({"path": target.to_string_lossy()}),
            ctx.clone(),
        )
        .await
        .expect("a read-only link is readable");
    let err = registry
        .dispatch(
            "fuzzy_patch",
            serde_json::json!({
                "path": target.to_string_lossy(),
                "search": "pub fn linked() {}",
                "replace": "x",
            }),
            ctx.clone(),
        )
        .await
        .expect_err("a read-only link is not writable")
        .to_string();
    assert!(err.contains("read-only"), "{err}");
}
