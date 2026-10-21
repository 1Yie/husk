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
