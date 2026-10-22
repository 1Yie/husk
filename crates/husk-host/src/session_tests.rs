//! Reading rules that survive a model switch — the parts of `session.rs` that
//! are pure enough to pin without an actor.

use super::*;

/// The prompt size describes the history; the cache half describes the cache
/// of one model. A switch keeps the former and drops the latter.
#[test]
fn a_model_switch_drops_the_cache_half_of_the_reading() {
    let reading = Some((12_000, 400, 9_600, 2_400));
    assert_eq!(
        cache_reading_after_switch(reading, false),
        reading,
        "same model: the reading still describes it"
    );
    assert_eq!(
        cache_reading_after_switch(reading, true),
        Some((12_000, 400, 0, 0)),
        "another model has another cache — nothing is known about it yet"
    );
    assert_eq!(
        cache_reading_after_switch(None, true),
        None,
        "nothing to trim"
    );
}

/// The `<!-- linked -->` region: the model has to know a linked file is
/// reachable before it tries, and a session with no grants spends nothing on
/// the concept.
#[test]
fn the_linked_block_names_each_grant_and_its_access() {
    let block = linked_roots_block(&[
        agent_capabilities::LinkedRoot {
            path: std::path::PathBuf::from("/w/kernel"),
            access: agent_capabilities::RootAccess::ReadWrite,
        },
        agent_capabilities::LinkedRoot {
            path: std::path::PathBuf::from("/w/docs"),
            access: agent_capabilities::RootAccess::Read,
        },
    ]);
    assert!(block.contains("/w/kernel  (read-write)"), "{block}");
    assert!(block.contains("/w/docs  (read-only)"), "{block}");
    assert!(block.contains("absolute path"), "{block}");

    assert!(linked_roots_block(&[]).is_empty(), "no grants, no block");
}

/// The re-rendered tail starts at whichever marker comes first, so a prompt
/// carrying both sections never keeps a copy of either.
#[test]
fn the_tail_starts_at_the_first_marker() {
    let both = "head\n\n## User instructions — project (/a)\n\nx\n\n## Rules\n\nr";
    assert_eq!(prompt_tail_start(both), Some("head".len()));
    // Rules without any AGENTS.md have no instructions marker — the rules
    // marker still anchors the truncation.
    assert_eq!(
        prompt_tail_start("head\n\n## Rules\n\nr"),
        Some("head".len())
    );
    assert_eq!(prompt_tail_start("head"), None);
}

/// A resume re-renders the whole tail: a rule edited since the session was
/// persisted must reach the reopened conversation, and a deleted one must not
/// survive as a stale constraint.
#[test]
fn a_resume_re_renders_the_rules_tail() {
    fn resume(prompt: &str, root: &std::path::Path, paths: &agent_paths::AppPaths) -> String {
        let mut out = prompt.to_string();
        if let Some(idx) = prompt_tail_start(&out) {
            out.truncate(idx);
        }
        append_prompt_tail(&mut out, root, paths);
        out
    }

    let dir = tempfile::tempdir().unwrap();
    let paths = agent_paths::AppPaths::new(dir.path().join("husk"), dir.path(), ".husk");
    let rules_dir = crate::rules::workspace_rules_dir(dir.path(), &paths);
    std::fs::create_dir_all(&rules_dir).unwrap();
    std::fs::write(
        rules_dir.join("style.md"),
        "---\ndescription: d\n---\nALWAYS X\n",
    )
    .unwrap();

    let mut first = String::from("head");
    append_prompt_tail(&mut first, dir.path(), &paths);
    assert!(first.contains("ALWAYS X"), "{first}");
    assert!(
        !first.contains("## User instructions"),
        "no AGENTS.md here: {first}"
    );

    // The rule is edited while the prompt on disk still has the old body.
    std::fs::write(
        rules_dir.join("style.md"),
        "---\ndescription: d\n---\nALWAYS Y\n",
    )
    .unwrap();
    let resumed = resume(&first, dir.path(), &paths);
    assert!(resumed.contains("ALWAYS Y"), "{resumed}");
    assert!(!resumed.contains("ALWAYS X"), "{resumed}");
    assert_eq!(resumed.matches("## Rules").count(), 1, "{resumed}");

    // With an AGENTS.md section too, the earlier marker wins: one copy each.
    std::fs::write(dir.path().join("AGENTS.md"), "PROJECT RULES\n").unwrap();
    let both = resume(&resumed, dir.path(), &paths);
    assert_eq!(both.matches("## User instructions").count(), 1, "{both}");
    assert_eq!(both.matches("## Rules").count(), 1, "{both}");
    assert!(both.contains("PROJECT RULES"), "{both}");

    // Deleted rule files drop the section on the next resume, and the
    // instructions section survives it.
    std::fs::remove_dir_all(&rules_dir).unwrap();
    let gone = resume(&both, dir.path(), &paths);
    assert!(!gone.contains("## Rules"), "{gone}");
    assert!(gone.contains("PROJECT RULES"), "{gone}");
}
