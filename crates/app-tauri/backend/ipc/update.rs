//! In-app updates from the GitHub Releases of this repo.
//!
//! `tauri-plugin-updater` is not usable here: it needs a signing key plus a
//! `latest.json` that the release workflow does not publish, and its `zip`
//! dependency is yanked on crates.io. The release already carries the
//! installers `tauri-action` builds, so this module picks the one matching
//! the running platform, downloads it with progress events, verifies it and
//! hands it to the system installer.
//!
//! The webview CSP blocks off-origin `fetch`, so every network call lives here.
//! The asset to install is remembered by `check_update`; `install_update`
//! takes no URL from the frontend.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use futures::StreamExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tauri::{AppHandle, Emitter as _, State};
use tokio::io::AsyncWriteExt as _;

const LATEST: &str = "https://api.github.com/repos/1Yie/husk/releases/latest";
/// Only assets served from this repo's releases are ever downloaded.
const DOWNLOAD_PREFIX: &str = "https://github.com/1Yie/husk/releases/download/";
pub const PROGRESS_EVENT: &str = "update-progress";

#[derive(Debug, Clone, Serialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    #[serde(skip)]
    pub url: String,
    #[serde(skip)]
    pub sha256: Option<String>,
}

#[derive(Default)]
pub struct UpdateState {
    asset: Mutex<Option<Asset>>,
    busy: AtomicBool,
    /// Windows only: the downloaded installer waiting for `relaunch_app`. Its
    /// installer must replace files this process holds, so it cannot run
    /// before the user confirms the restart.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pending: Mutex<Option<PathBuf>>,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    size: u64,
    browser_download_url: String,
    /// `sha256:<hex>` — present on current GitHub API responses.
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Clone, Serialize)]
struct Progress {
    /// `download` | `verify` | `install`
    phase: &'static str,
    downloaded: u64,
    total: u64,
}

fn emit(app: &AppHandle, phase: &'static str, downloaded: u64, total: u64) {
    let _ = app.emit(
        PROGRESS_EVENT,
        Progress {
            phase,
            downloaded,
            total,
        },
    );
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("husk/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())
}

/// Query the newest published release, remember the installer matching this
/// platform, and report whether it is newer than the running build.
///
/// Returns `{ current, latest, url, notes, is_newer, asset }`. `latest`,
/// `url` and `notes` are `null` when the release is a draft/prerelease;
/// `asset` is `null` when no installer fits this platform (the UI then links
/// to the release page instead of offering an in-app install).
#[tauri::command]
pub async fn check_update(state: State<'_, UpdateState>) -> Result<serde_json::Value, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let res = client()?
        .get(LATEST)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        // Surface the status so the UI says "检查失败 (404)" rather than
        // silently reading as "up to date".
        return Err(format!("GitHub 返回 {}", res.status()));
    }
    let rel: Release = res.json().await.map_err(|e| e.to_string())?;
    if rel.draft || rel.prerelease {
        return Ok(serde_json::json!({
            "current": current, "latest": null, "url": null, "notes": null,
            "is_newer": false, "asset": null
        }));
    }

    let latest = rel.tag_name.trim_start_matches('v').to_string();
    let newer = is_newer(&current, &latest);
    let asset = if newer {
        let assets: Vec<Asset> = rel
            .assets
            .into_iter()
            .filter(|a| a.browser_download_url.starts_with(DOWNLOAD_PREFIX))
            .map(|a| Asset {
                name: a.name,
                size: a.size,
                url: a.browser_download_url,
                sha256: a
                    .digest
                    .and_then(|d| d.strip_prefix("sha256:").map(str::to_lowercase)),
            })
            .collect();
        pick_asset(&assets, std::env::consts::OS, std::env::consts::ARCH, linux_uses_rpm())
    } else {
        None
    };
    *state.asset.lock().unwrap_or_else(|e| e.into_inner()) = asset.clone();
    Ok(serde_json::json!({
        "current": current,
        "latest": latest,
        "url": rel.html_url,
        "notes": rel.body,
        "is_newer": newer,
        "asset": asset,
    }))
}

/// Download, verify and install the asset chosen by the last `check_update`.
/// Returns once the update is ready; the app keeps running until the user
/// confirms with [`relaunch_app`]. Progress goes out on [`PROGRESS_EVENT`].
#[tauri::command]
pub async fn install_update(app: AppHandle, state: State<'_, UpdateState>) -> Result<(), String> {
    let asset = state
        .asset
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or("没有可安装的更新，请先检查更新")?;
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("更新正在进行".into());
    }
    let result = run_install(&app, &asset, &state).await;
    state.busy.store(false, Ordering::SeqCst);
    result
}

async fn run_install(app: &AppHandle, asset: &Asset, state: &UpdateState) -> Result<(), String> {
    let path = download(asset, |done, total| emit(app, "download", done, total)).await?;
    emit(app, "verify", asset.size, asset.size);
    verify(&path, asset).await?;
    emit(app, "install", asset.size, asset.size);
    stage_or_install(&path, state).await
}

/// Windows: keep the verified installer for [`relaunch_app`].
#[cfg(target_os = "windows")]
async fn stage_or_install(path: &Path, state: &UpdateState) -> Result<(), String> {
    *state.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(path.to_path_buf());
    Ok(())
}

/// Linux / macOS: the package is installed in place while the app keeps running.
#[cfg(not(target_os = "windows"))]
async fn stage_or_install(path: &Path, _state: &UpdateState) -> Result<(), String> {
    install_package(path).await
}

/// Restart into the new version. Call only after `install_update` succeeded.
#[tauri::command]
pub async fn relaunch_app(app: AppHandle, state: State<'_, UpdateState>) -> Result<(), String> {
    if state.busy.load(Ordering::SeqCst) {
        return Err("更新尚未完成".into());
    }
    #[cfg(target_os = "windows")]
    {
        let path = state
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .ok_or("没有已准备好的更新")?;
        // The installer replaces files this process holds open, so it starts
        // now and this process exits right after; it relaunches Husk itself.
        start_windows_installer(&path)?;
    }
    relaunch(&app);
    Ok(())
}

async fn download(asset: &Asset, progress: impl Fn(u64, u64)) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("husk-update");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("无法创建下载目录: {e}"))?;
    // The name came from the release; keep only its last component.
    let name = Path::new(&asset.name)
        .file_name()
        .ok_or("安装包文件名无效")?;
    let path = dir.join(name);
    let res = client()?
        .get(&asset.url)
        .send()
        .await
        .map_err(|e| format!("下载失败: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("下载失败: GitHub 返回 {}", res.status()));
    }
    let total = res.content_length().unwrap_or(asset.size);
    let mut file = tokio::fs::File::create(&path)
        .await
        .map_err(|e| format!("无法写入安装包: {e}"))?;
    let mut stream = res.bytes_stream();
    let (mut done, mut last_sent) = (0u64, 0u64);
    progress(0, total);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("下载中断: {e}"))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("无法写入安装包: {e}"))?;
        done += chunk.len() as u64;
        // ~0.5% steps keep the IPC quiet on a fast link.
        if done - last_sent >= (total / 200).max(64 * 1024) {
            last_sent = done;
            progress(done, total);
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    progress(done, total);
    if asset.size > 0 && done != asset.size {
        return Err(format!("安装包不完整: 应为 {} 字节，实际 {done}", asset.size));
    }
    Ok(path)
}

/// Compare against GitHub's published digest when it gave one.
async fn verify(path: &Path, asset: &Asset) -> Result<(), String> {
    let Some(expected) = asset.sha256.clone() else {
        return Ok(());
    };
    let path = path.to_owned();
    let actual = tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        let mut file = std::fs::File::open(path)?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher)?;
        Ok(hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("无法校验安装包: {e}"))?;
    if actual != expected {
        return Err("安装包校验失败（SHA-256 不一致），已放弃安装".into());
    }
    Ok(())
}

/// True on a Linux host that installs `.rpm` rather than `.deb` packages.
///
/// The distribution family decides, not the tools on PATH: Fedora can have
/// `dpkg` installed for unrelated reasons and must still get the `.rpm`.
fn linux_uses_rpm() -> bool {
    cfg!(target_os = "linux")
        && std::fs::read_to_string("/etc/os-release")
            .map(|text| os_release_is_rpm_family(&text))
            .unwrap_or_else(|_| which("dpkg").is_none() && which("rpm").is_some())
}

fn os_release_is_rpm_family(text: &str) -> bool {
    const RPM: [&str; 8] = [
        "fedora", "rhel", "centos", "rocky", "almalinux", "suse", "opensuse", "ol",
    ];
    const DEB: [&str; 3] = ["debian", "ubuntu", "linuxmint"];
    let mut ids: Vec<String> = Vec::new();
    for line in text.lines() {
        if let Some((key, value)) = line.split_once('=') {
            if key == "ID" || key == "ID_LIKE" {
                ids.extend(
                    value
                        .trim_matches('"')
                        .split_whitespace()
                        .map(str::to_lowercase),
                );
            }
        }
    }
    // Debian-likes win a tie: derivatives list `debian` in ID_LIKE.
    !ids.iter().any(|id| DEB.contains(&id.as_str()))
        && ids.iter().any(|id| RPM.contains(&id.as_str()) || id.starts_with("opensuse"))
}

fn which(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(program))
            .find(|candidate| candidate.is_file())
    })
}

/// Pick the release asset that fits `os`/`arch` (Rust's `std::env::consts`).
fn pick_asset(assets: &[Asset], os: &str, arch: &str, linux_rpm: bool) -> Option<Asset> {
    let arm = arch == "aarch64";
    let find = |matches: &dyn Fn(&str) -> bool| {
        assets
            .iter()
            .find(|a| matches(&a.name.to_lowercase()))
            .cloned()
    };
    match os {
        "macos" => find(&|n| {
            n.ends_with(".dmg") && (n.contains("aarch64") || n.contains("arm64")) == arm
        }),
        "windows" => {
            let arch_ok = |n: &str| (n.contains("arm64") || n.contains("aarch64")) == arm;
            find(&|n| n.ends_with("-setup.exe") && arch_ok(n))
                .or_else(|| find(&|n| n.ends_with(".msi") && arch_ok(n)))
        }
        "linux" => {
            let ext = if linux_rpm { ".rpm" } else { ".deb" };
            let arch_names: &[&str] = match (linux_rpm, arm) {
                (false, false) => &["amd64"],
                (false, true) => &["arm64"],
                (true, false) => &["x86_64"],
                (true, true) => &["aarch64"],
            };
            find(&|n| n.ends_with(ext) && arch_names.iter().any(|a| n.contains(a)))
        }
        _ => None,
    }
}

#[cfg(target_os = "linux")]
async fn install_package(path: &Path) -> Result<(), String> {
    let file = path.to_string_lossy().into_owned();
    // pkexec raises the desktop's own authentication prompt. apt/dnf resolve
    // the package's dependencies; dpkg/rpm are the bare fallbacks.
    let command: Vec<&str> = if file.ends_with(".rpm") {
        if which("dnf").is_some() {
            vec!["dnf", "install", "-y", &file]
        } else {
            vec!["rpm", "-U", "--force", &file]
        }
    } else if which("apt-get").is_some() {
        vec!["env", "DEBIAN_FRONTEND=noninteractive", "apt-get", "install", "-y", &file]
    } else {
        vec!["dpkg", "-i", &file]
    };
    if which("pkexec").is_none() {
        return Err(format!("缺少 pkexec，无法授权安装。请手动安装: {file}"));
    }
    let output = tokio::process::Command::new("pkexec")
        .args(&command)
        .output()
        .await
        .map_err(|e| format!("无法启动安装程序: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    // 126 = auth dismissed, 127 = auth failed (polkit convention).
    let reason = match output.status.code() {
        Some(126) => "已取消授权".to_string(),
        Some(127) => "授权失败".to_string(),
        _ => String::from_utf8_lossy(&output.stderr)
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("安装程序返回错误")
            .to_string(),
    };
    Err(format!("安装失败: {reason}。安装包保存在 {file}"))
}

#[cfg(target_os = "windows")]
fn start_windows_installer(path: &Path) -> Result<(), String> {
    // NSIS relaunches Husk itself (`/R`) once the files are replaced.
    let is_msi = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("msi"));
    let mut command = if is_msi {
        let mut c = std::process::Command::new("msiexec");
        c.arg("/i").arg(path).args(["/passive", "/norestart"]);
        c
    } else {
        let mut c = std::process::Command::new(path);
        c.args(["/P", "/R", "/UPDATE"]);
        c
    };
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("无法启动安装程序: {e}"))
}

#[cfg(target_os = "macos")]
async fn install_package(path: &Path) -> Result<(), String> {
    let bundle = running_bundle().ok_or("找不到当前应用包，无法原地更新")?;
    let mount = std::env::temp_dir().join("husk-update").join("mount");
    let _ = tokio::fs::create_dir_all(&mount).await;
    run("hdiutil", &["attach", "-nobrowse", "-readonly", "-noverify", "-mountpoint"], Some(&mount), Some(path)).await?;
    let swapped = async {
        let mut entries = tokio::fs::read_dir(&mount)
            .await
            .map_err(|e| e.to_string())?;
        let mut app = None;
        while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
            if entry.path().extension().is_some_and(|e| e == "app") {
                app = Some(entry.path());
                break;
            }
        }
        let app = app.ok_or("安装镜像里没有应用")?;
        let staged = bundle.with_extension("app.new");
        let _ = tokio::fs::remove_dir_all(&staged).await;
        // `ditto` keeps signatures, symlinks and extended attributes intact.
        run("ditto", &[], Some(&app), Some(&staged)).await?;
        tokio::fs::remove_dir_all(&bundle)
            .await
            .map_err(|e| format!("无法替换 {}: {e}", bundle.display()))?;
        tokio::fs::rename(&staged, &bundle)
            .await
            .map_err(|e| format!("无法替换 {}: {e}", bundle.display()))
    }
    .await;
    let _ = run("hdiutil", &["detach", "-quiet"], Some(&mount), None).await;
    swapped.map_err(|e| format!("安装失败: {e}。安装包保存在 {}", path.display()))
}

/// `program args… [first] [second]` with a readable error.
#[cfg(target_os = "macos")]
async fn run(
    program: &str,
    args: &[&str],
    first: Option<&Path>,
    second: Option<&Path>,
) -> Result<(), String> {
    let mut command = tokio::process::Command::new(program);
    command.args(args).args(first).args(second);
    let output = command
        .output()
        .await
        .map_err(|e| format!("{program}: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{program}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// `…/Husk.app` for the running executable (`…/Husk.app/Contents/MacOS/husk`).
#[cfg(target_os = "macos")]
fn running_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf)
}

/// Start the freshly installed build and quit this one. The delay lets this
/// process release the single-instance lock first, otherwise the new one would
/// exit immediately and only focus the old window.
#[cfg(not(target_os = "windows"))]
fn relaunch(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let target = running_bundle();
    // A package upgrade replaces the binary, so Linux reports the old inode as
    // "<path> (deleted)".
    #[cfg(not(target_os = "macos"))]
    let target = std::env::current_exe().ok().map(|p| {
        let text = p.to_string_lossy();
        PathBuf::from(text.strip_suffix(" (deleted)").unwrap_or(&text))
    });
    if let Some(target) = target {
        let script = if cfg!(target_os = "macos") {
            "sleep 2; exec open -n \"$0\""
        } else {
            "sleep 2; exec \"$0\""
        };
        let _ = std::process::Command::new("sh")
            .args(["-c", script])
            .arg(target)
            .spawn();
    }
    app.exit(0);
}

#[cfg(target_os = "windows")]
fn relaunch(app: &AppHandle) {
    app.exit(0);
}

/// Strict `a.b.c` numeric compare — `is_newer("0.1.9", "0.1.10")` is true.
/// Non-numeric segments (e.g. a `-rc1` suffix) make the comparison bail to
/// `false` rather than guess wrong about ordering.
fn is_newer(current: &str, latest: &str) -> bool {
    fn parts(v: &str) -> Option<Vec<u64>> {
        v.split('-')
            .next()?
            .split('.')
            .map(|p| p.parse().ok())
            .collect()
    }
    match (parts(current), parts(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    /// A tiny HTTP server answering every request with `body`.
    async fn serve(body: Vec<u8>) -> String {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let _ = socket.read(&mut buf).await;
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    for chunk in body.chunks(16 * 1024) {
                        let _ = socket.write_all(chunk).await;
                    }
                });
            }
        });
        format!("http://{addr}/pkg")
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    fn asset_for(url: String, body: &[u8], name: &str) -> Asset {
        Asset {
            name: name.into(),
            size: body.len() as u64,
            url,
            sha256: Some(sha256_hex(body)),
        }
    }

    fn assets() -> Vec<Asset> {
        [
            "Husk-0.3.0-1.x86_64.rpm",
            "Husk_0.3.0_aarch64.dmg",
            "Husk_0.3.0_amd64.deb",
            "Husk_0.3.0_x64-setup.exe",
            "Husk_0.3.0_x64_en-US.msi",
            "Husk_0.3.0_x64.dmg",
        ]
        .into_iter()
        .map(|name| Asset {
            name: name.into(),
            size: 1,
            url: String::new(),
            sha256: None,
        })
        .collect()
    }

    fn pick(os: &str, arch: &str, rpm: bool) -> Option<String> {
        pick_asset(&assets(), os, arch, rpm).map(|a| a.name)
    }

    #[test]
    fn picks_the_installer_for_each_platform() {
        assert_eq!(pick("macos", "aarch64", false).as_deref(), Some("Husk_0.3.0_aarch64.dmg"));
        assert_eq!(pick("macos", "x86_64", false).as_deref(), Some("Husk_0.3.0_x64.dmg"));
        assert_eq!(pick("windows", "x86_64", false).as_deref(), Some("Husk_0.3.0_x64-setup.exe"));
        assert_eq!(pick("linux", "x86_64", false).as_deref(), Some("Husk_0.3.0_amd64.deb"));
        assert_eq!(pick("linux", "x86_64", true).as_deref(), Some("Husk-0.3.0-1.x86_64.rpm"));
    }

    #[test]
    fn no_installer_for_an_unpublished_target() {
        assert_eq!(pick("linux", "aarch64", false), None);
        assert_eq!(pick("windows", "aarch64", false), None);
        assert_eq!(pick("freebsd", "x86_64", false), None);
    }

    #[test]
    fn windows_falls_back_to_msi_without_a_setup_exe() {
        let only_msi: Vec<_> = assets().into_iter().filter(|a| a.name.ends_with(".msi")).collect();
        let picked = pick_asset(&only_msi, "windows", "x86_64", false).unwrap();
        assert_eq!(picked.name, "Husk_0.3.0_x64_en-US.msi");
    }

    #[tokio::test]
    async fn downloads_reports_progress_and_verifies_the_digest() {
        let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let asset = asset_for(serve(body.clone()).await, &body, "husk-test-ok.deb");
        let last = AtomicU64::new(0);
        let path = download(&asset, |done, total| {
            assert_eq!(total, body.len() as u64);
            assert!(done >= last.swap(done, Ordering::SeqCst), "progress went backwards");
        })
        .await
        .unwrap();
        assert_eq!(last.load(Ordering::SeqCst), body.len() as u64);
        assert_eq!(std::fs::read(&path).unwrap(), body);
        verify(&path, &asset).await.unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn a_tampered_package_fails_verification() {
        let body = vec![7u8; 4096];
        let mut asset = asset_for(serve(body.clone()).await, &body, "husk-test-bad.deb");
        asset.sha256 = Some(sha256_hex(b"something else"));
        let path = download(&asset, |_, _| {}).await.unwrap();
        let error = verify(&path, &asset).await.unwrap_err();
        assert!(error.contains("校验失败"), "{error}");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn a_truncated_download_is_rejected() {
        let body = vec![1u8; 4096];
        let mut asset = asset_for(serve(body.clone()).await, &body, "husk-test-short.deb");
        asset.size = 8192;
        let error = download(&asset, |_, _| {}).await.unwrap_err();
        assert!(error.contains("不完整"), "{error}");
        let _ = std::fs::remove_file(std::env::temp_dir().join("husk-update/husk-test-short.deb"));
    }

    #[tokio::test]
    async fn the_asset_name_cannot_escape_the_download_directory() {
        let body = vec![2u8; 128];
        let asset = asset_for(serve(body.clone()).await, &body, "../../escape.deb");
        let path = download(&asset, |_, _| {}).await.unwrap();
        assert_eq!(path, std::env::temp_dir().join("husk-update/escape.deb"));
        let _ = std::fs::remove_file(path);
    }

    /// Hits the real GitHub release of this repo (network). Run explicitly:
    /// `cargo test -p husk live_release -- --ignored`
    #[tokio::test]
    #[ignore = "needs network"]
    async fn live_release_assets_match_every_platform_and_carry_digests() {
        let res = client()
            .unwrap()
            .get(LATEST)
            .send()
            .await
            .unwrap();
        assert!(res.status().is_success(), "{}", res.status());
        let rel: Release = res.json().await.unwrap();
        let assets: Vec<Asset> = rel
            .assets
            .into_iter()
            .filter(|a| a.browser_download_url.starts_with(DOWNLOAD_PREFIX))
            .map(|a| Asset {
                name: a.name,
                size: a.size,
                url: a.browser_download_url,
                sha256: a.digest.and_then(|d| d.strip_prefix("sha256:").map(str::to_lowercase)),
            })
            .collect();
        for (os, arch, rpm) in [
            ("macos", "aarch64", false),
            ("macos", "x86_64", false),
            ("windows", "x86_64", false),
            ("linux", "x86_64", false),
            ("linux", "x86_64", true),
        ] {
            let picked = pick_asset(&assets, os, arch, rpm)
                .unwrap_or_else(|| panic!("no installer for {os}/{arch} rpm={rpm}"));
            assert!(picked.sha256.is_some(), "{} has no digest", picked.name);
            assert!(picked.size > 1_000_000, "{} is suspiciously small", picked.name);
        }
    }

    #[test]
    fn package_family_follows_the_distribution() {
        assert!(os_release_is_rpm_family("NAME=\"Fedora Linux\"\nID=fedora\n"));
        assert!(os_release_is_rpm_family("ID=\"rhel\"\nID_LIKE=\"fedora\"\n"));
        assert!(os_release_is_rpm_family("ID=opensuse-tumbleweed\nID_LIKE=\"opensuse suse\"\n"));
        assert!(!os_release_is_rpm_family("ID=ubuntu\nID_LIKE=debian\n"));
        assert!(!os_release_is_rpm_family("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n"));
        assert!(!os_release_is_rpm_family("ID=arch\n"));
    }

    #[test]
    fn version_compare_is_numeric_and_strict() {
        assert!(is_newer("0.1.9", "0.1.10"));
        assert!(!is_newer("0.3.0", "0.3.0"));
        assert!(!is_newer("0.3.1", "0.3.0"));
        // A pre-release suffix is ignored; a non-numeric core bails to false.
        assert!(is_newer("0.3.0", "0.4.0-rc1"));
        assert!(!is_newer("0.3.0", "x.y.z"));
    }
}
