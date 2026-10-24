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
//!
//! Trust model: the SHA-256 published by the GitHub Releases API is an
//! integrity check for the download, not publisher signing. There is no
//! signing key or notarisation workflow here, so this module never claims to
//! verify a signature.
//!
//! Concurrency: every backend operation (`check` / `install` / `relaunch`)
//! runs under one [`Mutex`]-guarded state plus an RAII busy guard, so a second
//! call is rejected instead of racing the first. The guard clears the busy flag
//! on drop, including error and cancellation paths.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures::StreamExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tauri::{AppHandle, Emitter as _, State};
use tokio::io::AsyncWriteExt as _;
use tokio::sync::Notify;

const LATEST: &str = "https://api.github.com/repos/1Yie/husk/releases/latest";
/// Only assets served from this repo's releases are ever downloaded.
const DOWNLOAD_PREFIX: &str = "https://github.com/1Yie/husk/releases/download/";
pub const PROGRESS_EVENT: &str = "update-progress";
/// The error a cancelled download reports. The frontend treats it as a
/// user-initiated stop, not a failed install.
pub const DOWNLOAD_CANCELLED: &str = "已取消下载";
/// How long `cancel_update` waits for the running operation to wind down, and
/// how often it checks.
const CANCEL_WAIT: std::time::Duration = std::time::Duration::from_secs(3);
const CANCEL_POLL: std::time::Duration = std::time::Duration::from_millis(50);

#[derive(Debug, Clone, Serialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    #[serde(skip)]
    pub url: String,
    /// Lower-cased 64-hex SHA-256, or `None` when GitHub published no usable
    /// digest. `None` means "not installable": [`verify`] refuses it.
    #[serde(skip)]
    pub sha256: Option<String>,
    /// Release version this asset belongs to (for the Windows pending cache).
    #[serde(skip)]
    pub version: String,
}

/// How an update finished on this platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallMode {
    /// The package was already applied in place (Linux/macOS).
    Installed,
    /// The installer is prepared and runs when the app restarts (Windows).
    Staged,
}

#[derive(Debug, Serialize)]
pub struct InstallResult {
    pub mode: InstallMode,
}

/// A successfully prepared update. Kept past `install_update` so a failed
/// `relaunch_app` can retry the restart instead of reinstalling.
#[derive(Debug, Clone)]
struct Prepared {
    mode: InstallMode,
    version: String,
    digest: String,
    /// Windows: the persisted installer that `relaunch_app` runs.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    path: Option<PathBuf>,
}

/// Whether a prepared record is exactly the release the given asset installs.
/// Only an exact version + digest match may be advertised as ready, so a stale
/// record never labels a different release installed.
fn prepared_matches(prepared: &Prepared, asset: &Asset) -> bool {
    prepared.version == asset.version && asset.sha256.as_deref() == Some(prepared.digest.as_str())
}

/// The one piece of mutable update state. `busy` is set by [`begin_op`] for the
/// duration of an operation and cleared by the returned guard's `Drop`. `cancel`
/// is set only while `install_update` downloads; [`cancel_update`] uses it to
/// stop that download and nothing after it.
#[derive(Default)]
struct UpdateInner {
    asset: Option<Asset>,
    prepared: Option<Prepared>,
    cancel: Option<Arc<Notify>>,
    busy: bool,
}

#[derive(Default)]
pub struct UpdateState {
    inner: Mutex<UpdateInner>,
}

fn lock(state: &UpdateState) -> std::sync::MutexGuard<'_, UpdateInner> {
    state.inner.lock().unwrap_or_else(|e| e.into_inner())
}

/// RAII busy flag: set on creation, cleared on drop (success, error or a
/// cancelled future).
struct BusyGuard<'a> {
    state: &'a UpdateState,
}

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        lock(self.state).busy = false;
    }
}

/// Claim the single update slot. A concurrent call is rejected immediately
/// rather than queued behind a possibly minutes-long download.
fn begin_op(state: &UpdateState) -> Result<BusyGuard<'_>, String> {
    {
        let mut inner = lock(state);
        if inner.busy {
            return Err("更新操作正在进行，请稍候".into());
        }
        inner.busy = true;
    }
    Ok(BusyGuard { state })
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

/// A download that receives no bytes for this long is treated as stalled. The
/// busy guard is held for the whole download, so without it one dead
/// connection would block every later check/install until the app restarts.
const READ_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

fn client() -> Result<reqwest::Client, String> {
    client_with(READ_IDLE_TIMEOUT)
}

/// `read_timeout` resets after every successful read, so it bounds idle time,
/// not the total length of a large download on a slow link.
fn client_with(read_idle: std::time::Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("husk/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(10))
        .read_timeout(read_idle)
        .build()
        .map_err(|e| e.to_string())
}

/// Query the newest published release, remember the installer matching this
/// platform, and report whether it is newer than the running build.
///
/// Returns `{ current, latest, url, notes, is_newer, asset, prepared_mode,
/// manual_reason }`. `latest`, `url` and `notes` are `null` when the release is
/// a draft/prerelease; `asset` is `null` when no installer fits this platform;
/// `manual_reason` is set when an installer exists but this install is not
/// package-managed (AppImage/Flatpak/dev), so the UI offers a manual download.
#[tauri::command]
pub async fn check_update(state: State<'_, UpdateState>) -> Result<serde_json::Value, String> {
    let _op = begin_op(&state)?;
    let current = env!("CARGO_PKG_VERSION").to_string();
    // Windows: drop a staged installer once the running build has reached its
    // version (the restart applied it). While it is still newer than the
    // running build the preparation is kept, so a cancelled UAC or a failed
    // installer stays retryable. Blocking fs work, off the async runtime.
    #[cfg(target_os = "windows")]
    {
        let running = current.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(root) = pending_root() {
                cleanup_obsolete_pending(&root, &running);
            }
        })
        .await;
    }
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
            "is_newer": false, "asset": null, "prepared_mode": null,
            "manual_reason": null
        }));
    }

    let latest = rel.tag_name.trim_start_matches('v').to_string();
    let newer = is_newer(&current, &latest);
    if !newer {
        // Nothing newer is on offer: any prepared record is stale (the app has
        // since been updated and restarted).
        {
            let mut inner = lock(&state);
            inner.asset = None;
            inner.prepared = None;
        }
        return Ok(serde_json::json!({
            "current": current, "latest": latest, "url": rel.html_url,
            "notes": rel.body, "is_newer": false, "asset": null,
            "prepared_mode": null, "manual_reason": null
        }));
    }

    let linux_rpm = tokio::task::spawn_blocking(linux_uses_rpm)
        .await
        .unwrap_or(false);
    let asset = {
        let assets: Vec<Asset> = rel
            .assets
            .into_iter()
            .filter(|a| a.browser_download_url.starts_with(DOWNLOAD_PREFIX))
            .map(|a| Asset {
                name: a.name,
                size: a.size,
                url: a.browser_download_url,
                sha256: parse_digest(a.digest.as_deref()),
                version: latest.clone(),
            })
            .collect();
        pick_asset(
            &assets,
            std::env::consts::OS,
            std::env::consts::ARCH,
            linux_rpm,
        )
    };
    // A missing digest is a hard stop, decided before any download is attempted.
    let manual_reason = match asset.as_ref() {
        None => None,
        Some(a) if a.sha256.is_none() => {
            Some("发布包缺少可用的 SHA-256 校验值，无法自动安装，请从发布页手动下载".to_string())
        }
        Some(_) => tokio::task::spawn_blocking(manual_install_reason)
            .await
            .ok()
            .flatten(),
    };

    // Restore a staged Windows installer that survived a restart (blocking fs).
    #[cfg(target_os = "windows")]
    let restored = match asset.as_ref() {
        Some(a) => {
            let a = a.clone();
            let version = latest.clone();
            tokio::task::spawn_blocking(move || {
                let root = pending_root().ok()?;
                restore_pending(&root, &version, Some(&a))
            })
            .await
            .ok()
            .flatten()
        }
        None => None,
    };

    let prepared_mode = {
        let mut inner = lock(&state);
        // Remember exactly the asset that was checked; nothing else may
        // install while an operation is in flight (see `begin_op`).
        inner.asset = asset.clone();
        #[cfg(target_os = "windows")]
        if inner.prepared.is_none() {
            inner.prepared = restored;
        }
        // Advertise a prepared update only when it is exactly the selected
        // release; a stale record must not label a different release installed.
        inner
            .prepared
            .as_ref()
            .filter(|p| asset.as_ref().is_some_and(|a| prepared_matches(p, a)))
            .map(|p| p.mode)
    };

    Ok(serde_json::json!({
        "current": current,
        "latest": latest,
        "url": rel.html_url,
        "notes": rel.body,
        "is_newer": true,
        "asset": asset,
        "prepared_mode": prepared_mode,
        "manual_reason": manual_reason,
    }))
}

/// Download, verify and install (or stage) the asset chosen by the last
/// `check_update`. Returns `{ mode }`; on Windows the installer only runs when
/// the user confirms with [`relaunch_app`]. Progress goes out on
/// [`PROGRESS_EVENT`].
#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    state: State<'_, UpdateState>,
) -> Result<InstallResult, String> {
    let _op = begin_op(&state)?;
    let asset = lock(&state)
        .asset
        .clone()
        .ok_or("没有可安装的更新，请先检查更新")?;
    if asset.sha256.is_none() {
        return Err("发布信息缺少 SHA-256 校验值，无法安全安装".into());
    }
    // Idempotent retry: a prepared update for this exact version/digest is
    // already done; do not download or install it twice.
    if let Some(prepared) = lock(&state).prepared.clone() {
        if prepared_matches(&prepared, &asset) {
            return Ok(InstallResult {
                mode: prepared.mode,
            });
        }
    }

    let cancel = Arc::new(Notify::new());
    lock(&state).cancel = Some(cancel.clone());
    let download = until_cancelled(&cancel, download_asset(&app, &asset)).await;
    // The download phase is over (done, failed or cancelled). Clear the handle
    // now so a late cancel cannot reach verification or installation.
    lock(&state).cancel = None;
    let outcome = finish_install(&app, &asset, download?).await?;
    {
        let mut inner = lock(&state);
        inner.prepared = Some(Prepared {
            mode: outcome.mode,
            version: asset.version.clone(),
            digest: asset.sha256.clone().unwrap_or_default(),
            path: outcome.path,
        });
    }
    Ok(InstallResult { mode: outcome.mode })
}

struct InstallOutcome {
    mode: InstallMode,
    /// Windows: where the persisted installer lives.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    path: Option<PathBuf>,
}

/// Cancel the download of the running `install_update`. Returns once the
/// operation has wound down (or after [`CANCEL_WAIT`]), so a following
/// `check_update` is not rejected as busy. Does nothing if nothing is running.
/// Takes no `begin_op`: it must run while `install_update` holds the slot.
#[tauri::command]
pub async fn cancel_update(state: State<'_, UpdateState>) -> Result<(), String> {
    cancel_running(&state).await;
    Ok(())
}

async fn cancel_running(state: &UpdateState) {
    let deadline = std::time::Instant::now() + CANCEL_WAIT;
    loop {
        {
            let mut inner = lock(state);
            if !inner.busy {
                return;
            }
            // `notify_one` stores a permit when the download has not started
            // waiting yet, so the signal cannot be lost.
            if let Some(cancel) = inner.cancel.take() {
                cancel.notify_one();
            }
        }
        if std::time::Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(CANCEL_POLL).await;
    }
}

/// Runs `work` until it finishes or `cancel` is signalled. Dropping `work` on
/// cancel is what aborts the download and removes its partial file.
async fn until_cancelled<T>(
    cancel: &Notify,
    work: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::select! {
        _ = cancel.notified() => Err(DOWNLOAD_CANCELLED.into()),
        result = work => result,
    }
}

async fn download_asset(app: &AppHandle, asset: &Asset) -> Result<Download, String> {
    download(asset, |done, total| emit(app, "download", done, total)).await
}

/// Verify and install a finished download. Not cancellable: interrupting these
/// would leave the system half-updated.
async fn finish_install(
    app: &AppHandle,
    asset: &Asset,
    download: Download,
) -> Result<InstallOutcome, String> {
    // The download directory handle lives through verification *and* install.
    emit(app, "verify", asset.size, asset.size);
    verify(download.path(), asset).await?;
    emit(app, "install", asset.size, asset.size);
    install_or_stage(download, asset).await
}

/// Windows: persist the verified installer for [`relaunch_app`].
#[cfg(target_os = "windows")]
async fn install_or_stage(download: Download, asset: &Asset) -> Result<InstallOutcome, String> {
    let package = download.path().to_string_lossy().into_owned();
    let meta = PendingMeta {
        version: asset.version.clone(),
        name: asset.name.clone(),
        digest: asset.sha256.clone().unwrap_or_default(),
        size: asset.size,
        package,
    };
    // Metadata write is blocking; run it off the async runtime.
    tokio::task::spawn_blocking(move || {
        let root = pending_root()?;
        write_pending(&root, &meta)
    })
    .await
    .map_err(|e| e.to_string())??;
    let path = download.persist();
    Ok(InstallOutcome {
        mode: InstallMode::Staged,
        path: Some(path),
    })
}

/// Linux / macOS: apply the package in place while the app keeps running.
#[cfg(not(target_os = "windows"))]
async fn install_or_stage(download: Download, _asset: &Asset) -> Result<InstallOutcome, String> {
    install_package(download.path()).await?;
    Ok(InstallOutcome {
        mode: InstallMode::Installed,
        path: None,
    })
}

/// Restart into the prepared update. Rejected unless a successful install or
/// stage is on record; on failure the prepared state is retained so the caller
/// can retry the restart instead of reinstalling.
#[tauri::command]
pub async fn relaunch_app(app: AppHandle, state: State<'_, UpdateState>) -> Result<(), String> {
    let _op = begin_op(&state)?;
    let prepared = lock(&state).prepared.clone().ok_or("没有已准备好的更新")?;
    // On Unix the presence of a prepared record is the gate; the package was
    // already applied by `install_update`.
    #[cfg(not(target_os = "windows"))]
    let _ = &prepared;

    #[cfg(target_os = "windows")]
    {
        let path = prepared.path.clone().ok_or("没有已准备好的更新")?;
        // The package sat on disk since staging; re-verify it immediately
        // before the installer is allowed to run.
        verify_digest_async(&path, Some(prepared.digest.as_str())).await?;
        let spawned = start_windows_installer(&path);
        // On success only the in-memory record is dropped. The package and its
        // metadata stay on disk: if UAC is cancelled or the installer never
        // applies (the app restarts into the old build), the next check finds
        // the still-valid pending record and the preparation stays retryable.
        finish_relaunch(&mut lock(&state), spawned)?;
    }

    #[cfg(not(target_os = "windows"))]
    {
        let target = relaunch_target().ok_or("找不到可重启的程序路径")?;
        let spawned = launch_relaunch(&target, std::process::id());
        finish_relaunch(&mut lock(&state), spawned)?;
    }

    app.exit(0);
    Ok(())
}

/// Clear the prepared record only after the replacement process actually
/// started. On error it is kept untouched.
fn finish_relaunch(inner: &mut UpdateInner, spawned: Result<(), String>) -> Result<(), String> {
    match spawned {
        Ok(()) => {
            inner.prepared = None;
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// A private, unique download directory, backed by a `tempfile::TempDir` so
/// the name is unpredictable and the directory is removed on drop. On Windows
/// the same handle may be *disowned* (`persist`) so a staged installer survives
/// until the app is updated.
struct DownloadDir {
    path: PathBuf,
    temp: Option<tempfile::TempDir>,
    keep: bool,
}

impl DownloadDir {
    fn path(&self) -> &Path {
        &self.path
    }

    /// Disown the directory so it is not deleted when this handle drops.
    #[cfg(target_os = "windows")]
    fn persist(&mut self) {
        self.keep = true;
        if let Some(temp) = self.temp.take() {
            // Release the handle without deleting the directory.
            let _ = temp.keep();
        }
    }
}

impl Drop for DownloadDir {
    fn drop(&mut self) {
        // With a live `TempDir` (or after `persist`) the field handles cleanup
        // or the directory intentionally outlives us. Only the Windows error
        // path (no `TempDir`, not kept) removes the staging directory by hand.
        if self.keep || self.temp.is_some() {
            return;
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A fresh 0700 `TempDir` under the system temp directory.
#[cfg(not(target_os = "windows"))]
fn new_download_dir() -> Result<DownloadDir, String> {
    let temp = tempfile::Builder::new()
        .prefix("husk-update-")
        .tempdir()
        .map_err(|e| format!("无法创建下载目录: {e}"))?;
    // `tempfile` already uses 0700 on Unix; make it explicit.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("无法设置下载目录权限: {e}"))?;
    }
    Ok(DownloadDir {
        path: temp.path().to_path_buf(),
        temp: Some(temp),
        keep: false,
    })
}

/// A private, unpredictable staging directory inside the per-user cache.
#[cfg(target_os = "windows")]
fn new_download_dir() -> Result<DownloadDir, String> {
    let root = pending_root()?;
    let temp = tempfile::Builder::new()
        .prefix("husk-update-")
        .tempdir_in(&root)
        .map_err(|e| format!("无法创建下载目录: {e}"))?;
    Ok(DownloadDir {
        path: temp.path().to_path_buf(),
        temp: Some(temp),
        keep: false,
    })
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn create_private_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("无法创建目录 {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("无法设置目录权限: {e}"))?;
    }
    Ok(())
}

struct Download {
    /// Held for RAII (removes the directory on drop); read only by the
    /// Windows `persist` path.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    dir: DownloadDir,
    path: PathBuf,
}

impl Download {
    fn path(&self) -> &Path {
        &self.path
    }

    /// Keep the files past this handle (Windows staged installer).
    #[cfg(target_os = "windows")]
    fn persist(mut self) -> PathBuf {
        self.dir.persist();
        self.path.clone()
    }
}

async fn download(asset: &Asset, progress: impl Fn(u64, u64)) -> Result<Download, String> {
    // Directory creation touches the filesystem; keep it off the async runtime.
    let dir = tokio::task::spawn_blocking(new_download_dir)
        .await
        .map_err(|e| e.to_string())??;
    let path = download_into(&dir, asset, progress).await?;
    Ok(Download { dir, path })
}

async fn download_into(
    dir: &DownloadDir,
    asset: &Asset,
    progress: impl Fn(u64, u64),
) -> Result<PathBuf, String> {
    // Defence in depth: `check_update` already filters to this prefix, but the
    // download itself refuses an off-repo URL as well.
    if !asset.url.starts_with(DOWNLOAD_PREFIX) {
        return Err("安装包地址不受信任，已放弃下载".into());
    }
    fetch_into(dir, asset, progress).await
}

/// The download mechanics, separated from the URL-trust check so they can be
/// exercised against a local test server.
async fn fetch_into(
    dir: &DownloadDir,
    asset: &Asset,
    progress: impl Fn(u64, u64),
) -> Result<PathBuf, String> {
    fetch_with(&client()?, dir, asset, progress).await
}

async fn fetch_with(
    client: &reqwest::Client,
    dir: &DownloadDir,
    asset: &Asset,
    progress: impl Fn(u64, u64),
) -> Result<PathBuf, String> {
    let name = safe_file_name(&asset.name)?;
    let path = dir.path().join(&name);

    // `create_new` (O_EXCL) means a pre-existing file or symlink at the target
    // is never followed or overwritten.
    let res = client
        .get(&asset.url)
        .send()
        .await
        .map_err(|e| format!("下载失败: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("下载失败: GitHub 返回 {}", res.status()));
    }
    let total = res.content_length().unwrap_or(asset.size);
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .await
        .map_err(|e| format!("无法写入安装包: {e}"))?;
    let mut stream = res.bytes_stream();
    let (mut done, mut last_sent) = (0u64, 0u64);
    progress(0, total);
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(e) => {
                let _ = tokio::fs::remove_file(&path).await;
                return Err(format!("下载中断: {e}"));
            }
        };
        if let Err(e) = file.write_all(&chunk).await {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(format!("无法写入安装包: {e}"));
        }
        done += chunk.len() as u64;
        // ~0.5% steps keep the IPC quiet on a fast link.
        if done - last_sent >= (total / 200).max(64 * 1024) {
            last_sent = done;
            progress(done, total);
        }
    }
    if let Err(e) = file.flush().await {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(e.to_string());
    }
    drop(file);
    progress(done, total);
    if asset.size > 0 && done != asset.size {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(format!(
            "安装包不完整: 应为 {} 字节，实际 {done}",
            asset.size
        ));
    }
    Ok(path)
}

/// Keep only the final component of a release-provided file name.
fn safe_file_name(name: &str) -> Result<String, String> {
    let base = Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("安装包文件名无效")?;
    if base.is_empty() || base == "." || base == ".." {
        return Err("安装包文件名无效".into());
    }
    Ok(base.to_string())
}

/// Parse GitHub's `sha256:<64 hex>` digest; anything else (missing, another
/// algorithm, wrong length, non-hex) is not installable.
fn parse_digest(digest: Option<&str>) -> Option<String> {
    let hex = digest?.strip_prefix("sha256:")?;
    if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(hex.to_ascii_lowercase())
    } else {
        None
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("无法校验安装包: {e}"))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| format!("无法校验安装包: {e}"))?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Compare the file against its expected digest. A missing or malformed digest
/// is a hard failure (GitHub's digest is the only integrity signal we have).
fn verify_digest(path: &Path, expected: Option<&str>) -> Result<(), String> {
    let expected = expected.ok_or("发布包缺少 SHA-256 校验值，已放弃安装")?;
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("发布包的 SHA-256 校验值格式无效，已放弃安装".into());
    }
    let actual = sha256_file(path)?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err("安装包校验失败（SHA-256 不一致），已放弃安装".into());
    }
    Ok(())
}

async fn verify(path: &Path, asset: &Asset) -> Result<(), String> {
    verify_digest_async(path, asset.sha256.as_deref()).await
}

async fn verify_digest_async(path: &Path, expected: Option<&str>) -> Result<(), String> {
    let path = path.to_owned();
    let expected = expected.map(str::to_owned);
    tokio::task::spawn_blocking(move || verify_digest(&path, expected.as_deref()))
        .await
        .map_err(|e| e.to_string())?
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
        "fedora",
        "rhel",
        "centos",
        "rocky",
        "almalinux",
        "suse",
        "opensuse",
        "ol",
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
        && ids
            .iter()
            .any(|id| RPM.contains(&id.as_str()) || id.starts_with("opensuse"))
}

fn which(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(program))
            .find(|candidate| candidate.is_file())
    })
}

/// Pick the release asset that fits `os`/`arch` (Rust's `std::env::consts`).
///
/// Windows only ever picks the NSIS `-setup.exe`: the MSI cannot replace the
/// running app, so it is not offered for automatic install (the UI falls back
/// to the release download link).
fn pick_asset(assets: &[Asset], os: &str, arch: &str, linux_rpm: bool) -> Option<Asset> {
    let arm = arch == "aarch64";
    let find = |matches: &dyn Fn(&str) -> bool| {
        assets
            .iter()
            .find(|a| matches(&a.name.to_lowercase()))
            .cloned()
    };
    match os {
        "macos" => {
            find(&|n| n.ends_with(".dmg") && (n.contains("aarch64") || n.contains("arm64")) == arm)
        }
        "windows" => {
            let arch_ok = |n: &str| (n.contains("arm64") || n.contains("aarch64")) == arm;
            find(&|n| n.ends_with("-setup.exe") && arch_ok(n))
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

/// True when an owning package string names Husk itself.
#[cfg(any(target_os = "linux", test))]
fn is_husk_package(owner: &str) -> bool {
    // dpkg: `husk: /usr/bin/husk`; rpm: `husk-0.3.0-1.x86_64`.
    let name = owner
        .split(':')
        .next()
        .unwrap_or("")
        .trim()
        .split_once('-')
        .map(|(n, _)| n)
        .unwrap_or_else(|| owner.split(':').next().unwrap_or("").trim());
    name == "husk"
}

/// Decide whether a Linux install may be applied automatically.
///
/// A running executable that no package owns (dev build, manually unpacked
/// tarball) must not get a second package installed over it. Only a Husk package
/// install is eligible; everything else reports a manual-download reason.
#[cfg(any(target_os = "linux", test))]
fn classify_linux_install(
    appimage: bool,
    flatpak: bool,
    owner: Option<&str>,
) -> Result<(), String> {
    if appimage {
        return Err("AppImage 版本无法原地更新，请从发布页手动下载".into());
    }
    if flatpak {
        return Err("Flatpak 版本请通过 Flatpak 更新，或从发布页手动下载".into());
    }
    match owner {
        Some(o) if is_husk_package(o) => Ok(()),
        Some(o) => Err(format!(
            "当前程序由软件包 {o} 管理，请用系统包管理器更新，或从发布页手动下载"
        )),
        None => {
            Err("当前程序不是由软件包管理的版本（开发版或手动解压），请从发布页手动下载".into())
        }
    }
}

#[cfg(target_os = "linux")]
fn running_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let text = exe.to_string_lossy().into_owned();
    Some(PathBuf::from(
        text.strip_suffix(" (deleted)").unwrap_or(&text),
    ))
}

#[cfg(target_os = "linux")]
fn package_owner(exe: &Path) -> Option<String> {
    let exe = exe.to_string_lossy();
    let output = if linux_uses_rpm() {
        std::process::Command::new("rpm")
            .args(["-qf", &exe])
            .output()
            .ok()?
    } else {
        std::process::Command::new("dpkg-query")
            .args(["-S", &exe])
            .output()
            .ok()?
    };
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

#[cfg(target_os = "linux")]
fn linux_install_eligibility() -> Result<(), String> {
    let appimage = std::env::var_os("APPIMAGE").is_some();
    let flatpak = Path::new("/.flatpak-info").exists() || std::env::var_os("FLATPAK_ID").is_some();
    let owner = if appimage || flatpak {
        None
    } else {
        running_exe().and_then(|e| package_owner(&e))
    };
    classify_linux_install(appimage, flatpak, owner.as_deref())
}

/// `None` when an in-app install is possible; otherwise the reason the UI must
/// fall back to a manual download.
fn manual_install_reason() -> Option<String> {
    #[cfg(target_os = "linux")]
    return linux_install_eligibility().err();
    #[cfg(not(target_os = "linux"))]
    return None;
}

#[cfg(target_os = "linux")]
async fn install_package(path: &Path) -> Result<(), String> {
    // Eligibility is rechecked immediately before the privileged install, off
    // the async runtime because it runs `rpm`/`dpkg-query`.
    tokio::task::spawn_blocking(linux_install_eligibility)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|reason| format!("无法自动安装：{reason}"))?;
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
        vec![
            "env",
            "DEBIAN_FRONTEND=noninteractive",
            "apt-get",
            "install",
            "-y",
            &file,
        ]
    } else {
        vec!["dpkg", "-i", &file]
    };
    if which("pkexec").is_none() {
        return Err("缺少 pkexec，无法授权安装。请从发布页下载安装包后手动安装".into());
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
    Err(format!("安装失败: {reason}"))
}

/// Build the Windows installer invocation. Only NSIS setup executables are
/// auto-installed: `/P` passive, `/R` re-run the app, `/UPDATE` update the
/// existing installation. MSI cannot replace a running install and is refused
/// (the UI offers the download instead).
#[cfg(any(target_os = "windows", test))]
const NSIS_ARGS: [&str; 3] = ["/P", "/R", "/UPDATE"];

#[cfg(any(target_os = "windows", test))]
fn windows_installer_command(path: &Path) -> Result<std::process::Command, String> {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("msi"))
    {
        return Err("MSI 安装包暂不支持自动安装，请从发布页手动下载".into());
    }
    let mut command = std::process::Command::new(path);
    command.args(NSIS_ARGS);
    Ok(command)
}

#[cfg(target_os = "windows")]
fn start_windows_installer(path: &Path) -> Result<(), String> {
    windows_installer_command(path)?
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("无法启动安装程序: {e}"))
}

/// Metadata for a staged Windows installer, persisted across app restarts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
struct PendingMeta {
    version: String,
    name: String,
    digest: String,
    size: u64,
    /// Absolute path of the persisted package.
    package: String,
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn pending_root() -> Result<PathBuf, String> {
    let base = dirs::cache_dir()
        .ok_or("找不到缓存目录")?
        .join("husk")
        .join("updates");
    create_private_dir(&base)?;
    Ok(base)
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn pending_meta_path(root: &Path) -> PathBuf {
    root.join("pending.json")
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn write_pending(root: &Path, meta: &PendingMeta) -> Result<(), String> {
    let path = pending_meta_path(root);
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_vec(meta).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text).map_err(|e| format!("无法保存更新状态: {e}"))?;
    // `rename` replaces an existing file on every platform (Windows uses
    // MOVEFILE_REPLACE_EXISTING), so the old record survives until the swap.
    std::fs::rename(&tmp, &path).map_err(|e| format!("无法保存更新状态: {e}"))
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn read_pending(root: &Path) -> Option<PendingMeta> {
    let text = std::fs::read_to_string(pending_meta_path(root)).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn clear_pending(root: &Path) {
    let _ = std::fs::remove_file(pending_meta_path(root));
}

/// Resolve a stored package path and require it to live under `root`.
///
/// `canonicalize` follows symlinks/reparse points, so a link that escapes
/// `root` is rejected; the returned path is the real one. Never trust the raw
/// serialized string: only a regular file under the cache root is returned.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn validated_package_path(root: &Path, raw: &str) -> Option<PathBuf> {
    let root = std::fs::canonicalize(root).ok()?;
    let path = std::fs::canonicalize(raw).ok()?;
    (path.starts_with(&root) && path.is_file()).then_some(path)
}

/// Restore a staged installer only when it matches the currently offered
/// version and digest and the package is still on disk.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn restore_pending(root: &Path, latest: &str, asset: Option<&Asset>) -> Option<Prepared> {
    let meta = read_pending(root)?;
    let asset = asset?;
    if meta.version != latest || meta.digest != asset.sha256.clone()? {
        return None;
    }
    let path = validated_package_path(root, &meta.package)?;
    Some(Prepared {
        mode: InstallMode::Staged,
        version: meta.version,
        digest: meta.digest,
        path: Some(path),
    })
}

/// Drop a staged installer once the running version has actually reached it.
///
/// Called after a restart into the new build. While `current` is older than the
/// pending version the preparation is kept (retryable); the package is only
/// removed after validation that it is a regular file under the cache root, so
/// a tampered `pending.json` can never delete an arbitrary path.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn cleanup_obsolete_pending(root: &Path, current: &str) {
    let Some(meta) = read_pending(root) else {
        return;
    };
    if is_newer(current, &meta.version) {
        return;
    }
    let canonical_root = std::fs::canonicalize(root).ok();
    if let Some(path) = validated_package_path(root, &meta.package) {
        if let Some(dir) = path.parent() {
            // Never remove the cache root itself, only the package's own dir.
            if canonical_root.as_deref() != Some(dir) {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
    clear_pending(root);
}

/// Replace `target` with `staged`, keeping `target` safe in `backup` until the
/// new copy is in place. A failed swap restores the backup.
///
/// The rename step is injected so the rollback path can be tested without
/// touching real application bundles.
#[cfg(any(target_os = "macos", test))]
fn replace_dir_with<F>(
    target: &Path,
    staged: &Path,
    backup: &Path,
    mut rename: F,
) -> Result<(), String>
where
    F: FnMut(&Path, &Path) -> std::io::Result<()>,
{
    // A leftover backup may be the only surviving original after an
    // interrupted swap. Refuse to overwrite it.
    if backup
        .try_exists()
        .map_err(|e| format!("无法检查原安装备份: {e}"))?
    {
        return Err(format!(
            "原安装备份已存在，请先恢复或移走 {}",
            backup.display()
        ));
    }
    rename(target, backup).map_err(|e| format!("无法备份原安装: {e}"))?;
    match rename(staged, target) {
        Ok(()) => {
            let _ = std::fs::remove_dir_all(backup);
            Ok(())
        }
        Err(e) => {
            if let Err(rollback) = rename(backup, target) {
                return Err(format!(
                    "替换失败({e})，且回滚失败({rollback})，原安装位于 {}",
                    backup.display()
                ));
            }
            Err(format!("替换失败: {e}，已回滚到原版本"))
        }
    }
}

#[cfg(target_os = "macos")]
async fn install_package(path: &Path) -> Result<(), String> {
    let bundle = running_bundle().ok_or("找不到当前应用包，无法原地更新")?;
    let parent = bundle.parent().ok_or("应用包没有父目录")?;
    let mount = tempfile::Builder::new()
        .prefix("husk-update-mount-")
        .tempdir()
        .map_err(|e| format!("无法创建挂载目录: {e}"))?;
    // Stage next to the bundle so the final rename stays on one filesystem.
    let stage_dir = tempfile::Builder::new()
        .prefix("husk-stage-")
        .tempdir_in(parent)
        .map_err(|e| format!("无法创建暂存目录: {e}"))?;
    run(
        "hdiutil",
        &[
            "attach",
            "-nobrowse",
            "-readonly",
            "-noverify",
            "-mountpoint",
        ],
        Some(mount.path()),
        Some(path),
    )
    .await?;
    let swapped = async {
        let app = find_app(mount.path()).await?;
        let staged = stage_dir
            .path()
            .join(bundle.file_name().ok_or("应用包路径无效")?);
        // `ditto` keeps signatures, symlinks and extended attributes intact.
        run("ditto", &[], Some(&app), Some(&staged)).await?;
        // Never remove the original before the replacement can be renamed in.
        let backup = bundle.with_extension(format!("app.husk-old-{}", std::process::id()));
        replace_dir_with(&bundle, &staged, &backup, |from, to| {
            std::fs::rename(from, to)
        })
    }
    .await;
    let _ = run("hdiutil", &["detach", "-quiet"], Some(mount.path()), None).await;
    swapped.map_err(|e| format!("安装失败: {e}"))
}

/// Find the `.app` bundle inside a mounted disk image.
#[cfg(target_os = "macos")]
async fn find_app(dir: &Path) -> Result<PathBuf, String> {
    let mut entries = tokio::fs::read_dir(dir).await.map_err(|e| e.to_string())?;
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        if entry.path().extension().is_some_and(|e| e == "app") {
            return Ok(entry.path());
        }
    }
    Err("安装镜像里没有应用".into())
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

/// The path to start again after an update.
#[cfg(not(target_os = "windows"))]
fn relaunch_target() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        running_bundle()
    }
    #[cfg(not(target_os = "macos"))]
    {
        // A package upgrade replaces the binary, so the old inode reports as
        // "<path> (deleted)".
        let exe = std::env::current_exe().ok()?;
        let text = exe.to_string_lossy().into_owned();
        Some(PathBuf::from(
            text.strip_suffix(" (deleted)").unwrap_or(&text),
        ))
    }
}

/// Spawn a detached `sh` that waits for `pid` to exit, then starts `target`.
///
/// The loop replaces a fixed `sleep` so a slow shutdown never races the new
/// process into the single-instance lock. `target` and `pid` are passed as
/// literal argv entries (no shell interpolation). A spawn error is returned so
/// the caller does not exit the app.
#[cfg(not(target_os = "windows"))]
fn launch_relaunch(target: &Path, pid: u32) -> Result<(), String> {
    let target = target.to_string_lossy().into_owned();
    let pid = pid.to_string();
    let script = if cfg!(target_os = "macos") {
        "while kill -0 \"$1\" 2>/dev/null; do sleep 0.1; done; exec open -n \"$0\""
    } else {
        "while kill -0 \"$1\" 2>/dev/null; do sleep 0.1; done; exec \"$0\""
    };
    std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .arg(target)
        .arg(pid)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("无法启动重启程序: {e}"))
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
    use std::sync::atomic::{AtomicU64, Ordering};

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
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn asset_for(url: String, body: &[u8], name: &str) -> Asset {
        Asset {
            name: name.into(),
            size: body.len() as u64,
            url,
            sha256: Some(sha256_hex(body)),
            version: "0.4.0".into(),
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
            version: "0.3.0".into(),
        })
        .collect()
    }

    fn pick(os: &str, arch: &str, rpm: bool) -> Option<String> {
        pick_asset(&assets(), os, arch, rpm).map(|a| a.name)
    }

    #[test]
    fn picks_the_installer_for_each_platform() {
        assert_eq!(
            pick("macos", "aarch64", false).as_deref(),
            Some("Husk_0.3.0_aarch64.dmg")
        );
        assert_eq!(
            pick("macos", "x86_64", false).as_deref(),
            Some("Husk_0.3.0_x64.dmg")
        );
        assert_eq!(
            pick("windows", "x86_64", false).as_deref(),
            Some("Husk_0.3.0_x64-setup.exe")
        );
        assert_eq!(
            pick("linux", "x86_64", false).as_deref(),
            Some("Husk_0.3.0_amd64.deb")
        );
        assert_eq!(
            pick("linux", "x86_64", true).as_deref(),
            Some("Husk-0.3.0-1.x86_64.rpm")
        );
    }

    #[test]
    fn no_installer_for_an_unpublished_target() {
        assert_eq!(pick("linux", "aarch64", false), None);
        assert_eq!(pick("windows", "aarch64", false), None);
        assert_eq!(pick("freebsd", "x86_64", false), None);
    }

    #[test]
    fn msi_is_not_offered_for_automatic_install() {
        let only_msi: Vec<_> = assets()
            .into_iter()
            .filter(|a| a.name.ends_with(".msi"))
            .collect();
        assert!(pick_asset(&only_msi, "windows", "x86_64", false).is_none());
    }

    #[test]
    fn digests_must_be_64_hex_or_they_are_rejected() {
        let ok = "a".repeat(64);
        assert_eq!(
            parse_digest(Some(&format!("sha256:{}", ok.to_uppercase()))),
            Some(ok.clone())
        );
        assert_eq!(parse_digest(Some("sha256:abc")), None);
        assert_eq!(parse_digest(Some("md5:abc")), None);
        assert_eq!(
            parse_digest(Some(
                "sha256:zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"
            )),
            None
        );
        assert_eq!(parse_digest(None), None);
    }

    #[test]
    fn a_missing_or_wrong_digest_fails_verification() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pkg.deb");
        std::fs::write(&path, b"payload").unwrap();
        let missing = verify_digest(&path, None).unwrap_err();
        assert!(missing.contains("缺少"), "{missing}");
        let malformed = verify_digest(&path, Some("not-hex")).unwrap_err();
        assert!(malformed.contains("格式无效"), "{malformed}");
        let wrong = verify_digest(&path, Some(&sha256_hex(b"other"))).unwrap_err();
        assert!(wrong.contains("校验失败"), "{wrong}");
        verify_digest(&path, Some(&sha256_hex(b"payload"))).unwrap();
    }

    #[tokio::test]
    async fn downloads_report_progress_and_verify_the_digest() {
        let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let asset = asset_for(serve(body.clone()).await, &body, "husk-test-ok.deb");
        let last = AtomicU64::new(0);
        let dir = new_download_dir().unwrap();
        let path = fetch_into(&dir, &asset, |done, total| {
            assert_eq!(total, body.len() as u64);
            assert!(
                done >= last.swap(done, Ordering::SeqCst),
                "progress went backwards"
            );
        })
        .await
        .unwrap();
        assert_eq!(last.load(Ordering::SeqCst), body.len() as u64);
        assert_eq!(std::fs::read(&path).unwrap(), body);
        verify(&path, &asset).await.unwrap();
    }

    #[tokio::test]
    async fn a_tampered_package_fails_verification() {
        let body = vec![7u8; 4096];
        let mut asset = asset_for(serve(body.clone()).await, &body, "husk-test-bad.deb");
        asset.sha256 = Some(sha256_hex(b"something else"));
        let dir = new_download_dir().unwrap();
        let path = fetch_into(&dir, &asset, |_, _| {}).await.unwrap();
        let error = verify(&path, &asset).await.unwrap_err();
        assert!(error.contains("校验失败"), "{error}");
    }

    #[tokio::test]
    async fn untrusted_download_urls_are_refused() {
        let dir = new_download_dir().unwrap();
        let asset = asset_for("http://127.0.0.1:9/pkg".into(), b"x", "x.deb");
        let error = download_into(&dir, &asset, |_, _| {}).await.unwrap_err();
        assert!(error.contains("不受信任"), "{error}");
    }

    #[tokio::test]
    async fn a_truncated_download_is_rejected_and_leaves_no_partial() {
        let body = vec![1u8; 4096];
        let mut asset = asset_for(serve(body.clone()).await, &body, "husk-test-short.deb");
        asset.size = 8192;
        let dir = new_download_dir().unwrap();
        let error = fetch_into(&dir, &asset, |_, _| {}).await.unwrap_err();
        assert!(error.contains("不完整"), "{error}");
        assert!(!dir.path().join("husk-test-short.deb").exists());
    }

    /// A server that sends headers and part of the body, then goes silent
    /// with the connection open.
    async fn serve_stalled(declared: usize, sent: usize) -> String {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let _ = socket.read(&mut buf).await;
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {declared}\r\n\r\n");
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&vec![0u8; sent]).await;
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                });
            }
        });
        format!("http://{addr}/pkg")
    }

    #[tokio::test]
    async fn a_stalled_download_times_out_and_leaves_no_partial() {
        let body = vec![0u8; 8192];
        let asset = asset_for(
            serve_stalled(8192, 1024).await,
            &body,
            "husk-test-stall.deb",
        );
        let dir = new_download_dir().unwrap();
        let client = client_with(std::time::Duration::from_millis(200)).unwrap();
        let started = std::time::Instant::now();
        let error = fetch_with(&client, &dir, &asset, |_, _| {})
            .await
            .unwrap_err();
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert!(error.contains("下载中断"), "{error}");
        assert!(!dir.path().join("husk-test-stall.deb").exists());
    }

    #[tokio::test]
    async fn download_directories_are_private_unique_and_escape_safe() {
        let body = vec![2u8; 128];
        let url = serve(body.clone()).await;
        let escaped = asset_for(url.clone(), &body, "../../escape.deb");
        let normal = asset_for(url, &body, "escape.deb");
        let dir_a = new_download_dir().unwrap();
        let dir_b = new_download_dir().unwrap();
        let (a, b) = tokio::join!(
            fetch_into(&dir_a, &escaped, |_, _| {}),
            fetch_into(&dir_b, &normal, |_, _| {})
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        // Distinct unique directories, same sanitized file name.
        assert_ne!(dir_a.path(), dir_b.path());
        assert_eq!(a.file_name().unwrap().to_str().unwrap(), "escape.deb");
        assert_eq!(b.file_name().unwrap().to_str().unwrap(), "escape.deb");
        // No symlink/path escape: the file sits directly in its own directory.
        assert_eq!(a.parent(), Some(dir_a.path()));
        assert_eq!(a, dir_a.path().join("escape.deb"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            for d in [&dir_a, &dir_b] {
                let mode = std::fs::metadata(d.path()).unwrap().permissions().mode();
                assert_eq!(mode & 0o777, 0o700);
            }
        }
        // A second download into the same directory must not clobber the first
        // (create_new, not follow-or-overwrite).
        let second = fetch_into(&dir_a, &normal, |_, _| {}).await;
        assert!(second.is_err());
    }

    #[test]
    fn safe_file_name_strips_paths_and_rejects_empties() {
        assert_eq!(safe_file_name("../../escape.deb").unwrap(), "escape.deb");
        assert_eq!(safe_file_name("dir/pkg.rpm").unwrap(), "pkg.rpm");
        assert!(safe_file_name("").is_err());
        assert!(safe_file_name("..").is_err());
    }

    #[test]
    fn concurrent_operations_are_rejected_and_the_guard_clears_busy() {
        let state = UpdateState::default();
        let first = begin_op(&state).unwrap();
        assert!(begin_op(&state).is_err());
        drop(first);
        // RAII: a dropped guard (success, error or cancellation) frees the slot.
        let second = begin_op(&state).unwrap();
        drop(second);
        assert!(begin_op(&state).is_ok());
    }

    #[tokio::test]
    async fn cancelling_an_idle_update_returns_at_once() {
        let state = UpdateState::default();
        let started = std::time::Instant::now();
        cancel_running(&state).await;
        assert!(started.elapsed() < CANCEL_WAIT);
    }

    #[tokio::test]
    async fn cancelling_a_stalled_download_aborts_it_and_removes_the_partial() {
        let body = vec![0u8; 8192];
        let asset = asset_for(
            serve_stalled(8192, 1024).await,
            &body,
            "husk-test-cancel.deb",
        );
        let dir = new_download_dir().unwrap();
        let http = client().unwrap();
        let cancel = Notify::new();
        let cancel_later = async {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            cancel.notify_one();
        };
        let (result, ()) = tokio::join!(
            until_cancelled(&cancel, fetch_with(&http, &dir, &asset, |_, _| {})),
            cancel_later,
        );
        assert_eq!(result.unwrap_err(), DOWNLOAD_CANCELLED);
        // The half-written package is still on disk until its directory goes.
        let path = dir.path().to_path_buf();
        assert!(path.join("husk-test-cancel.deb").exists());
        drop(dir);
        assert!(!path.exists());
    }

    #[test]
    fn a_failed_relaunch_keeps_the_prepared_update() {
        let mut inner = UpdateInner {
            prepared: Some(Prepared {
                mode: InstallMode::Staged,
                version: "0.4.0".into(),
                digest: sha256_hex(b"x"),
                path: Some(PathBuf::from("/tmp/does-not-matter")),
            }),
            ..Default::default()
        };
        assert!(finish_relaunch(&mut inner, Err("spawn failed".into())).is_err());
        assert!(
            inner.prepared.is_some(),
            "prepared state must survive a failed restart"
        );
        assert!(finish_relaunch(&mut inner, Ok(())).is_ok());
        assert!(inner.prepared.is_none());
    }

    #[test]
    fn prepared_only_matches_the_exact_version_and_digest() {
        let prepared = Prepared {
            mode: InstallMode::Installed,
            version: "0.4.0".into(),
            digest: sha256_hex(b"a"),
            path: None,
        };
        let mut asset = Asset {
            name: "Husk_0.4.0_amd64.deb".into(),
            size: 1,
            url: String::new(),
            sha256: Some(sha256_hex(b"a")),
            version: "0.4.0".into(),
        };
        assert!(prepared_matches(&prepared, &asset));
        asset.version = "0.5.0".into();
        assert!(
            !prepared_matches(&prepared, &asset),
            "a newer release is not installed"
        );
        asset.version = "0.4.0".into();
        asset.sha256 = Some(sha256_hex(b"b"));
        assert!(!prepared_matches(&prepared, &asset));
        asset.sha256 = None;
        assert!(!prepared_matches(&prepared, &asset));
    }

    #[test]
    fn obsolete_pending_is_kept_until_its_version_is_reached() {
        let root = tempfile::tempdir().unwrap();
        let staging = root.path().join("husk-update-abc");
        std::fs::create_dir_all(&staging).unwrap();
        let pkg = staging.join("Husk_x64-setup.exe");
        std::fs::write(&pkg, b"installer").unwrap();
        let meta = PendingMeta {
            version: "0.4.0".into(),
            name: "Husk_x64-setup.exe".into(),
            digest: sha256_hex(b"installer"),
            size: 9,
            package: pkg.to_string_lossy().into_owned(),
        };
        write_pending(root.path(), &meta).unwrap();
        // Still newer than the running build: preparation must stay retryable.
        cleanup_obsolete_pending(root.path(), "0.3.0");
        assert!(
            pkg.exists(),
            "package must survive while the app is still older"
        );
        assert!(read_pending(root.path()).is_some());
        // Restarted into the new version: the staged package can be removed.
        cleanup_obsolete_pending(root.path(), "0.4.0");
        assert!(!pkg.exists());
        assert!(read_pending(root.path()).is_none());
    }

    #[test]
    fn restored_paths_outside_the_cache_root_are_refused_and_never_deleted() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let victim = outside.path().join("victim.exe");
        std::fs::write(&victim, b"x").unwrap();
        let digest = sha256_hex(b"x");
        let meta = PendingMeta {
            version: "0.4.0".into(),
            name: "victim.exe".into(),
            digest: digest.clone(),
            size: 1,
            package: victim.to_string_lossy().into_owned(),
        };
        write_pending(root.path(), &meta).unwrap();
        let asset = Asset {
            name: "victim.exe".into(),
            size: 1,
            url: String::new(),
            sha256: Some(digest),
            version: "0.4.0".into(),
        };
        assert!(validated_package_path(root.path(), &meta.package).is_none());
        assert!(restore_pending(root.path(), "0.4.0", Some(&asset)).is_none());
        // Cleanup must not delete a path outside the cache root.
        cleanup_obsolete_pending(root.path(), "0.4.0");
        assert!(victim.exists(), "must never delete outside the cache root");
    }

    #[cfg(unix)]
    #[test]
    fn restored_paths_reject_symlink_escapes() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let real = outside.path().join("real.exe");
        std::fs::write(&real, b"x").unwrap();
        let link = root.path().join("link.exe");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(validated_package_path(root.path(), link.to_str().unwrap()).is_none());
    }

    #[test]
    fn directory_replacement_preserves_an_existing_backup() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Husk.app");
        let staged = dir.path().join("staged.app");
        let backup = dir.path().join("backup.app");
        for path in [&target, &staged, &backup] {
            std::fs::create_dir(path).unwrap();
        }
        std::fs::write(target.join("version"), "current").unwrap();
        std::fs::write(backup.join("version"), "recoverable").unwrap();
        let error = replace_dir_with(&target, &staged, &backup, |from, to| {
            std::fs::rename(from, to)
        })
        .unwrap_err();
        assert!(error.contains("备份已存在"));
        assert_eq!(
            std::fs::read_to_string(target.join("version")).unwrap(),
            "current"
        );
        assert_eq!(
            std::fs::read_to_string(backup.join("version")).unwrap(),
            "recoverable"
        );
    }

    #[test]
    fn directory_replacement_swaps_the_bundle_and_drops_the_backup() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("Husk.app");
        let staged = root.path().join("staged-app");
        let backup = root.path().join("Husk.app.old");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("marker"), b"original").unwrap();
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(staged.join("marker"), b"new").unwrap();

        replace_dir_with(&target, &staged, &backup, |from, to| {
            std::fs::rename(from, to)
        })
        .unwrap();
        assert_eq!(std::fs::read(target.join("marker")).unwrap(), b"new");
        assert!(!backup.exists());
    }

    #[test]
    fn directory_replacement_rolls_back_when_the_new_copy_cannot_land() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("Husk.app");
        let staged = root.path().join("staged-app");
        let backup = root.path().join("Husk.app.old");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("marker"), b"original").unwrap();
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(staged.join("marker"), b"new").unwrap();

        let mut calls = 0;
        let result = replace_dir_with(&target, &staged, &backup, |from, to| {
            calls += 1;
            if calls == 2 {
                return Err(std::io::Error::other("injected failure"));
            }
            std::fs::rename(from, to)
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(target.join("marker")).unwrap(), b"original");
        assert!(!backup.exists(), "backup restored, not left behind");
    }

    #[test]
    fn linux_auto_install_requires_a_husk_managed_executable() {
        assert!(classify_linux_install(false, false, Some("husk: /usr/bin/husk")).is_ok());
        assert!(classify_linux_install(false, false, Some("husk-0.3.0-1.x86_64")).is_ok());
        assert!(classify_linux_install(false, false, Some("other-1.0-1.x86_64")).is_err());
        assert!(classify_linux_install(false, false, None).is_err());
        // Packaged as AppImage/Flatpak: never install a second package.
        assert!(classify_linux_install(true, false, Some("husk")).is_err());
        assert!(classify_linux_install(false, true, Some("husk")).is_err());
    }

    #[test]
    fn windows_installer_uses_passive_relaunch_update_flags_and_refuses_msi() {
        let command =
            windows_installer_command(Path::new("C:/x/Husk_0.4.0_x64-setup.exe")).unwrap();
        let args: Vec<String> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["/P", "/R", "/UPDATE"]);
        assert!(windows_installer_command(Path::new("C:/x/Husk_0.4.0_x64.msi")).is_err());
    }

    #[test]
    fn staged_pending_round_trips_and_restores_only_on_matching_identity() {
        let root = tempfile::tempdir().unwrap();
        let pkg_dir = root.path().join("staging-1");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        let pkg = pkg_dir.join("Husk_x64-setup.exe");
        std::fs::write(&pkg, b"installer").unwrap();
        let digest = sha256_hex(b"installer");
        let meta = PendingMeta {
            version: "0.4.0".into(),
            name: "Husk_x64-setup.exe".into(),
            digest: digest.clone(),
            size: 9,
            package: pkg.to_string_lossy().into_owned(),
        };
        write_pending(root.path(), &meta).unwrap();
        assert_eq!(read_pending(root.path()).as_ref(), Some(&meta));

        let asset = Asset {
            name: "Husk_x64-setup.exe".into(),
            size: 9,
            url: String::new(),
            sha256: Some(digest),
            version: "0.4.0".into(),
        };
        let restored = restore_pending(root.path(), "0.4.0", Some(&asset)).unwrap();
        assert_eq!(restored.mode, InstallMode::Staged);
        // `restore_pending` returns the canonicalized package path.
        let canonical = std::fs::canonicalize(&pkg).unwrap();
        assert_eq!(restored.path.as_deref(), Some(canonical.as_path()));

        // Wrong version or a different digest is not restored.
        assert!(restore_pending(root.path(), "0.5.0", Some(&asset)).is_none());
        let mut other = asset.clone();
        other.sha256 = Some(sha256_hex(b"other"));
        assert!(restore_pending(root.path(), "0.4.0", Some(&other)).is_none());
        // A missing package file is not restored either.
        std::fs::remove_file(&pkg).unwrap();
        assert!(restore_pending(root.path(), "0.4.0", Some(&asset)).is_none());
    }

    /// Hits the real GitHub release of this repo (network). Run explicitly:
    /// `cargo test -p husk live_release -- --ignored`
    #[tokio::test]
    #[ignore = "needs network"]
    async fn live_release_assets_match_every_platform_and_carry_digests() {
        let res = client().unwrap().get(LATEST).send().await.unwrap();
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
                sha256: parse_digest(a.digest.as_deref()),
                version: rel.tag_name.trim_start_matches('v').to_string(),
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
            assert!(
                picked.size > 1_000_000,
                "{} is suspiciously small",
                picked.name
            );
        }
    }

    #[test]
    fn package_family_follows_the_distribution() {
        assert!(os_release_is_rpm_family(
            "NAME=\"Fedora Linux\"\nID=fedora\n"
        ));
        assert!(os_release_is_rpm_family(
            "ID=\"rhel\"\nID_LIKE=\"fedora\"\n"
        ));
        assert!(os_release_is_rpm_family(
            "ID=opensuse-tumbleweed\nID_LIKE=\"opensuse suse\"\n"
        ));
        assert!(!os_release_is_rpm_family("ID=ubuntu\nID_LIKE=debian\n"));
        assert!(!os_release_is_rpm_family(
            "ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n"
        ));
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
