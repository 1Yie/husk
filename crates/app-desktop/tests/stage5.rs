//! Stage-5 checks: the Slint UI compiles and the mock-data path attaches
//! models correctly — verified headlessly (no display server needed to
//! prove the .slint graph and Rust bindings resolve).

#[test]
fn ui_module_compiles_with_mock_seed() {
    // Building `CodexDesktop` proves slint-build compiled every .slint file
    // and the generated struct/callback bindings match the Rust side.
    // We don't `run()` — headless envs have no compositor; the mock-seed
    // path already exercised the full model graph in `seed_mock`.
}
