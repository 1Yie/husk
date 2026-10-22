//! Cross-workspace grants: once a directory is linked, the file tools reach it
//! exactly as they reach the workspace — and the reach is bounded by the
//! access level, not by the tool.

use std::sync::Arc;

use agent_capabilities::{LinkedRoot, LinkedRoots, RootAccess, WorkspaceCtx};
use agent_kernel::tools::ToolCtx;

struct Fixture {
    _dir: tempfile::TempDir,
    ws: std::path::PathBuf,
    linked: std::path::PathBuf,
    other: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let [ws, linked, other] = ["app", "kernel", "outside"].map(|n| base.join(n));
    for d in [&ws, &linked, &other] {
        std::fs::create_dir_all(d).unwrap();
        std::fs::write(d.join("f.txt"), "line1\nline2\n").unwrap();
    }
    Fixture {
        _dir: dir,
        ws,
        linked,
        other,
    }
}

fn ctx(f: &Fixture, roots: Vec<LinkedRoot>) -> Arc<ToolCtx> {
    let handle = Arc::new(LinkedRoots::default());
    handle.set(roots);
    let mut ctx = ToolCtx::new().with_workspace(&f.ws);
    ctx.install_linked_roots(handle);
    Arc::new(ctx)
}

fn read_link(f: &Fixture, access: RootAccess) -> Vec<LinkedRoot> {
    vec![LinkedRoot {
        path: f.linked.clone(),
        access,
    }]
}

/// The point of the feature: a patch to another repository applies without any
/// extra ceremony once that repository is linked read-write.
#[tokio::test]
async fn a_read_write_link_takes_a_patch() {
    let f = fixture();
    let ctx = ctx(&f, read_link(&f, RootAccess::ReadWrite));
    let registry = husk_host::session::base_registries().0;
    let target = f.linked.join("f.txt");
    let args = serde_json::json!({
        "path": target.to_string_lossy(),
        "search": "line2",
        "replace": "line2 patched",
    });
    registry
        .dispatch("fuzzy_patch", args, ctx.clone())
        .await
        .expect("a linked read-write root is in scope")
        .commit
        .expect("staged")
        .apply
        .await
        .expect("applied");
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "line1\nline2 patched\n"
    );
}

/// Reads reach a read-only link; writes refuse with the reason, and the tool
/// never falls back to the workspace or to `bash`.
#[tokio::test]
async fn a_read_only_link_reads_but_refuses_writes() {
    let f = fixture();
    let ctx = ctx(&f, read_link(&f, RootAccess::Read));
    let registry = husk_host::session::base_registries().0;
    let target = f.linked.join("f.txt");
    let read = registry
        .dispatch(
            "smart_read",
            serde_json::json!({"path": target.to_string_lossy()}),
            ctx.clone(),
        )
        .await
        .expect("reads reach a read-only link");
    assert!(read.content.contains("line1"), "{}", read.content);

    let err = registry
        .dispatch(
            "fuzzy_patch",
            serde_json::json!({
                "path": target.to_string_lossy(),
                "search": "line2",
                "replace": "x",
            }),
            ctx.clone(),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("read-only"), "{err}");
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "line1\nline2\n",
        "nothing may be written"
    );
}

/// Nothing linked → the old refusal, unchanged. A link is a grant, not a
/// widening of the default.
#[tokio::test]
async fn an_unlinked_directory_still_escapes() {
    let f = fixture();
    let ctx = ctx(&f, read_link(&f, RootAccess::ReadWrite));
    let registry = husk_host::session::base_registries().0;
    let err = registry
        .dispatch(
            "smart_read",
            serde_json::json!({"path": f.other.join("f.txt").to_string_lossy()}),
            ctx,
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("path escapes workspace"), "{err}");
}

/// A delegated child inherits the grant (the slot is `insert_shared`), so a
/// subagent can finish a cross-repository task the parent started.
#[test]
fn a_child_context_inherits_the_grant_handle() {
    let f = fixture();
    let parent = ctx(&f, read_link(&f, RootAccess::ReadWrite));
    let child = ToolCtx {
        ext: parent.ext.inheritable(),
        ..ToolCtx::new()
    };
    let target = f.linked.join("f.txt");
    assert_eq!(
        child.resolve(&target.to_string_lossy()).unwrap(),
        target,
        "the child writes to a linked root"
    );
    // Revoking on the parent handle reaches the child too: same `Arc`.
    let handle = child.ext.get::<LinkedRoots>().expect("inherited");
    handle.set(Vec::new());
    assert!(child.resolve(&target.to_string_lossy()).is_err());
}


// ---- the workspace's grant hub ------------------------------------------

fn app_paths(f: &Fixture) -> agent_paths::AppPaths {
    agent_paths::AppPaths::new(f.ws.join(".config"), f.ws.join(".data"), ".husk")
}

fn handle_and_hub(
    f: &Fixture,
) -> (
    Arc<husk_host::links::LinkHub>,
    Arc<LinkedRoots>,
    Arc<LinkedRoots>,
) {
    let hub = husk_host::links::hub_for(&f.ws, &app_paths(f));
    let one = Arc::new(LinkedRoots::default());
    let two = Arc::new(LinkedRoots::default());
    for (id, handle) in [(1i64, &one), (2i64, &two)] {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        // No actor runs in these tests; the receiver is only a place for a
        // refresh's notice to land.
        std::mem::forget(rx);
        hub.register(id, handle, &tx);
    }
    (hub, one, two)
}

/// A grant reaches every session of the workspace, not only the one that made
/// it — and so does a revocation. A window that kept reaching a directory the
/// user just unlinked would be a permission the user believes they removed.
#[test]
fn a_grant_and_a_revocation_reach_every_live_session() {
    let f = fixture();
    let (hub, one, two) = handle_and_hub(&f);

    hub.grant(1, &f.linked.to_string_lossy(), "readwrite", husk_host::links::Layer::Workspace)
        .expect("granted");
    // Session 2 asked for nothing, yet the workspace-level grant is live in it.
    assert_eq!(
        two.snapshot().iter().map(|r| r.path.clone()).collect::<Vec<_>>(),
        vec![f.linked.canonicalize().unwrap()]
    );
    assert!(one.snapshot().iter().any(|r| r.access == RootAccess::ReadWrite));

    hub.revoke(2, &f.linked).expect("revoked");
    assert!(one.snapshot().is_empty(), "the other window lost it too");
    assert!(two.snapshot().is_empty());
    assert!(husk_host::links::hub_for(&f.ws, &app_paths(&f))
        .effective(1)
        .is_empty());
}

/// A stored grant is validated on the way back out. A prefs file edited by
/// hand (or written by an older build, or left by a directory that has since
/// moved) must not become reach — the hub is the only reader of that file.
#[test]
fn a_stored_grant_that_no_longer_validates_is_not_honoured() {
    let f = fixture();
    let store = agent_store::SessionStore::open(&f.ws).unwrap();
    let inside_workspace = f.ws.join("src");
    std::fs::create_dir_all(&inside_workspace).unwrap();
    let gone = f.other.join("moved-away");
    store
        .update_prefs(|prefs| {
            prefs.linked_roots = vec![
                // Inside the workspace: a link there grants nothing new, and
                // the policy refuses it outright.
                agent_store::LinkedRootPref {
                    path: inside_workspace,
                    access: "readwrite".into(),
                },
                // Drives are unmounted, projects are moved.
                agent_store::LinkedRootPref {
                    path: gone,
                    access: "readwrite".into(),
                },
                // The one legitimate entry of the file.
                agent_store::LinkedRootPref {
                    path: f.linked.clone(),
                    access: "read".into(),
                },
            ];
        })
        .unwrap();

    let hub = husk_host::links::hub_for(&f.ws, &app_paths(&f));
    let effective = hub.effective(1);
    assert_eq!(
        effective,
        vec![LinkedRoot {
            path: f.linked.canonicalize().unwrap(),
            access: RootAccess::Read,
        }],
        "only the entry that still validates is reach"
    );
    let (workspace, session) = hub.layers(1);
    assert_eq!(workspace.len(), 1);
    assert!(session.is_empty());
}

/// A row the user can see in the pane must be removable even when it no
/// longer grants anything — "nothing changed" would be a lie that leaves the
/// entry in the file forever.
#[test]
fn a_stale_row_can_still_be_unlinked() {
    let f = fixture();
    let store = agent_store::SessionStore::open(&f.ws).unwrap();
    let gone = f.other.join("moved-away");
    store
        .update_prefs(|prefs| {
            prefs.linked_roots = vec![agent_store::LinkedRootPref {
                path: gone.clone(),
                access: "readwrite".into(),
            }]
        })
        .unwrap();
    let hub = husk_host::links::hub_for(&f.ws, &app_paths(&f));
    assert!(hub.effective(1).is_empty(), "it grants nothing");

    hub.revoke(1, &gone).expect("removing the row succeeds");
    assert!(store.load_prefs().linked_roots.is_empty());
    // A path that was never granted is still an error, not a silent success.
    assert!(hub.revoke(1, &f.other).is_err());
}
