fn main() {
    // slint-build compiles ui/ — the single entry point resolves imports.
    slint_build::compile("ui/codex_window.slint").expect("slint compile failed");
}
