fn main() {
    // The webview assets are embedded from `frontend/dist`. Cargo does not
    // watch that directory, so `npm run build` alone left the PREVIOUS binary
    // in place and the app kept serving the old UI until an unrelated edit
    // forced a recompile.
    println!("cargo:rerun-if-changed=frontend/dist");
    emit_kernel_build_info();
    tauri_build::build()
}

/// The kernel this app is built on, read from the resolved dependency graph so
/// the About page can never drift from what was actually linked. Exposed as
/// `KOREI_VERSION` / `KOREI_REV` (short commit, empty for a path dependency).
fn emit_kernel_build_info() {
    println!("cargo:rerun-if-changed=../../Cargo.lock");
    let out = std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["metadata", "--format-version", "1", "--locked"])
        .output();
    let (mut version, mut rev) = (String::new(), String::new());
    if let Ok(out) = out {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&text) {
                let kernel = meta["packages"]
                    .as_array()
                    .and_then(|ps| ps.iter().find(|p| p["name"] == "agent-kernel"));
                if let Some(k) = kernel {
                    version = k["version"].as_str().unwrap_or_default().to_string();
                    // `git+ssh://…?branch=main#177dacd7…` — the fragment is the commit.
                    if let Some(src) = k["source"].as_str() {
                        if let Some((_, r)) = src.split_once('#') {
                            rev = r.chars().take(7).collect();
                        }
                    }
                }
            }
        }
    }
    println!("cargo:rustc-env=KOREI_VERSION={version}");
    println!("cargo:rustc-env=KOREI_REV={rev}");
}
