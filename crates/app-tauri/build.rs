fn main() {
    // The webview assets are embedded from `frontend/dist`. Cargo does not
    // watch that directory, so `npm run build` alone left the PREVIOUS binary
    // in place and the app kept serving the old UI until an unrelated edit
    // forced a recompile.
    println!("cargo:rerun-if-changed=frontend/dist");
    tauri_build::build()
}
