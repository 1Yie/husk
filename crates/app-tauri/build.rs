fn main() {
    // The webview assets are embedded from `frontend/dist`. Cargo does not
    // watch that directory, so `npm run build` alone left the PREVIOUS binary
    // in place and the app kept serving the old UI until an unrelated edit
    // forced a recompile.
    println!("cargo:rerun-if-changed=frontend/dist");
    link_compiler_rt_on_macos();
    tauri_build::build()
}

/// ONNX Runtime's CoreML objects call `__isPlatformVersionAtLeast`, which the
/// clang driver supplies from compiler-rt. A Rust binary on macOS links with
/// `-nodefaultlibs` (static CRT), so that archive is never added and the link
/// fails on the symbol. Ask clang where it keeps it and link it explicitly.
fn link_compiler_rt_on_macos() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let Ok(out) = std::process::Command::new("clang")
        .arg("-print-file-name=libclang_rt.osx.a")
        .output()
    else {
        return;
    };
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let archive = std::path::Path::new(&path);
    // A missing archive prints back as the bare name, which has no directory.
    let Some(dir) = archive.parent().filter(|_| archive.is_file()) else {
        return;
    };
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}

