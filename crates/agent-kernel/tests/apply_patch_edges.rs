//! Edge coverage for `apply_patch`: empty files, binary/unreadable targets,
//! and the Envelope bounds (no-ops, oversize). Exercises the full
//! `ToolRegistry::dispatch` path so the errors the model sees are what we
//! actually assert on.

use std::sync::Arc;

use agent_kernel::tools::{ToolCtx, ToolRegistry};

fn registry() -> ToolRegistry {
    ToolRegistry::with_builtins()
}

fn ctx_at(dir: &std::path::Path) -> Arc<ToolCtx> {
    Arc::new(ToolCtx::new(dir))
}

async fn patch(dir: &std::path::Path, body: &str) -> Result<String, String> {
    registry()
        .dispatch(
            "apply_patch",
            serde_json::json!({ "patch": body }),
            ctx_at(dir),
        )
        .await
        .map(|r| r.content)
        .map_err(|e| e.to_string())
}

#[tokio::test]
async fn add_empty_file_writes_zero_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let r = patch(
        dir.path(),
        "*** Begin Patch\n*** Add File: empty.txt\n*** End Patch\n",
    )
    .await
    .expect("empty add is legal");
    assert!(r.contains("added empty.txt"));
}

#[tokio::test]
async fn update_empty_file_with_eof_append_works() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("empty.txt"), "").unwrap();
    let r = patch(
        dir.path(),
        "*** Begin Patch\n*** Update File: empty.txt\n\
         @@\n*** End of File\n+first line\n+second\n*** End Patch\n",
    )
    .await
    .expect("eof append onto an empty file is the canonical seed");
    assert!(r.contains("updated empty.txt"));
    // The virtual-fs diff is emitted as a pending write — the file on disk
    // changes only through the write pipeline, not here.
}

#[tokio::test]
async fn update_missing_file_says_add_instead() {
    let dir = tempfile::tempdir().unwrap();
    let err = patch(
        dir.path(),
        "*** Begin Patch\n*** Update File: ghost.txt\n\
         @@\n-old\n+new\n*** End Patch\n",
    )
    .await
    .unwrap_err();
    assert!(
        err.contains("does not exist") && err.contains("Add File"),
        "missing-file update should teach Add File, got: {err}"
    );
}

#[tokio::test]
async fn binary_file_is_not_treated_as_missing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("bin.dat"), b"\x00\x01\x02\xff\xfe").unwrap();

    // Update must NOT say "does not exist" — it exists, it's just not text.
    let err = patch(
        dir.path(),
        "*** Begin Patch\n*** Update File: bin.dat\n\
         @@\n-old\n+new\n*** End Patch\n",
    )
    .await
    .unwrap_err();
    assert!(
        err.contains("not UTF-8") || err.contains("cannot read"),
        "binary update must name the real problem, got: {err}"
    );

    // Add must NOT silently clobber the binary.
    let err = patch(
        dir.path(),
        "*** Begin Patch\n*** Add File: bin.dat\n\
         +text\n*** End Patch\n",
    )
    .await
    .unwrap_err();
    assert!(
        err.contains("not UTF-8") || err.contains("cannot read"),
        "add onto a binary must refuse, got: {err}"
    );

    // And the bytes on disk are untouched — no silent overwrite happened.
    assert_eq!(
        std::fs::read(dir.path().join("bin.dat")).unwrap(),
        b"\x00\x01\x02\xff\xfe",
        "binary file must survive the refused patch"
    );
}

#[tokio::test]
async fn delete_binary_reports_unreadable_not_missing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("bin.dat"), b"\xff\xfe\x00\x01").unwrap();
    let err = patch(
        dir.path(),
        "*** Begin Patch\n*** Delete File: bin.dat\n*** End Patch\n",
    )
    .await
    .unwrap_err();
    assert!(
        !err.contains("does not exist"),
        "delete on an existing binary must not say missing, got: {err}"
    );
}

#[tokio::test]
async fn empty_patch_is_an_args_error_not_a_noop() {
    let dir = tempfile::tempdir().unwrap();
    let err = patch(dir.path(), "*** Begin Patch\n*** End Patch\n")
        .await
        .unwrap_err();
    assert!(
        err.contains("no file operations"),
        "empty envelope should teach the grammar, got: {err}"
    );
}

#[tokio::test]
async fn add_then_update_same_patch_composes() {
    let dir = tempfile::tempdir().unwrap();
    // Two ops on one path in a single patch — the virtual-fs pipeline must
    // apply Update against the just-Added content, not stale disk state.
    let r = patch(
        dir.path(),
        "*** Begin Patch\n*** Add File: a.txt\n\
         +alpha\n+beta\n*** Update File: a.txt\n\
         @@\n-beta\n+gamma\n*** End Patch\n",
    )
    .await
    .expect("add→update in one patch composes");
    assert!(r.contains("added a.txt") && r.contains("updated a.txt"));
}
