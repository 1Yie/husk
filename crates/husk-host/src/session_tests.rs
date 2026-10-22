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
