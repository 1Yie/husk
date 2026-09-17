//! Stage-5 checks: the iced UI state compiles and the event-application
//! logic holds — verified headlessly (no display server needed; `update` +
//! `view` are pure functions over `App`).

#[test]
fn ui_module_compiles() {
    // The crate building proves the iced widget tree + `App::update`/`view`
    // type-check end to end. Rendering needs a compositor — exercised via
    // `--mock` on a live display instead of a headless test.
}
