fn main() {
    // The webview assets are embedded from `frontend/dist`. Cargo does not
    // watch that directory, so `npm run build` alone left the PREVIOUS binary
    // in place and the app kept serving the old UI until an unrelated edit
    // forced a recompile.
    println!("cargo:rerun-if-changed=frontend/dist");
    link_compiler_rt_on_macos();
    rpath_bundled_onnxruntime_on_intel_mac();
    tauri_build::build()
}

/// ort-sys ships no prebuilt ONNX Runtime for x86_64-apple-darwin, so that
/// target links the dylib built by `scripts/build-onnxruntime-macos-x86_64.sh`
/// (install name `@rpath/libonnxruntime.1.dylib`). The bundle copies it into
/// `Contents/Frameworks`; ort-sys adds no rpath, so point the binary there.
fn rpath_bundled_onnxruntime_on_intel_mac() {
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    if var("CARGO_CFG_TARGET_OS") != "macos" || var("CARGO_CFG_TARGET_ARCH") != "x86_64" {
        return;
    }
    println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@executable_path/../Frameworks");
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

