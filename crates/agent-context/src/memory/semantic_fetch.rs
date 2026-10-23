//! Model files for the semantic encoder (`semantic` feature).
//!
//! The encoder's weights are fetched here rather than by `fastembed`'s own
//! hub client. That client does not read a proxy, trusts only a bundled root
//! store (so a corporate or a packet-inspecting proxy's CA is rejected),
//! carries no timeout, and promotes a resumed `.part` file without checking
//! it — one interrupted download left a corrupt blob that every later load
//! failed on, with the same opaque error. This module owns the transfer instead:
//! system roots, the environment's proxy (plus the system one on macOS),
//! timeouts, bounded retries, `Range` resume, and a size + sha256 check before
//! a file takes its final name.
//!
//! The cache is `<dir>/<repo>/<file>`. A verified file is remembered in a
//! `<file>.verified` stamp so a 235 MB weight is hashed once, not per launch.
//! A cache written by the old hub client is adopted when it verifies, so an
//! upgrade does not cost the user a second download.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use sha2::{Digest, Sha256};

/// The multilingual sentence encoder. 384 dims, quantized.
pub(crate) const REPO: &str = "Qdrant/paraphrase-multilingual-MiniLM-L12-v2-onnx-Q";
/// Hub revision — a branch name is fine: every file is hash-checked anyway.
const REVISION: &str = "main";
const DEFAULT_ENDPOINT: &str = "https://huggingface.co";
/// Reached for only after the primary endpoint fails.
const MIRROR_ENDPOINT: &str = "https://hf-mirror.com";

/// One file of the encoder, with what a download of it must add up to.
pub(crate) struct FileSpec {
    pub name: &'static str,
    pub size: u64,
    /// Present for the LFS blobs, whose sha256 the hub publishes. The small
    /// JSON sidecars are plain git objects with no sha256 to compare against;
    /// their exact size is the check, and a truncated one fails loudly in the
    /// tokenizer parser rather than silently.
    pub sha256: Option<&'static str>,
}

/// What `TextEmbedding` reads for this model: the graph plus the tokenizer's
/// four files. The sizes are the hub's own.
pub(crate) const FILES: &[FileSpec] = &[
    FileSpec {
        name: "model_optimized.onnx",
        size: 235_052_644,
        sha256: Some("634d0f66c29dc934c8fa72b8a4fe91dd4d420a22f1d82a241058d4316e659a99"),
    },
    FileSpec {
        name: "tokenizer.json",
        size: 17_083_009,
        sha256: Some("fa685fc160bbdbab64058d4fc91b60e62d207e8dc60b9af5c002c5ab946ded00"),
    },
    FileSpec {
        name: "config.json",
        size: 673,
        sha256: None,
    },
    FileSpec {
        name: "special_tokens_map.json",
        size: 964,
        sha256: None,
    },
    FileSpec {
        name: "tokenizer_config.json",
        size: 1_416,
        sha256: None,
    },
];

/// Total download size — the denominator of the progress bar.
pub(crate) fn total_bytes() -> u64 {
    FILES.iter().map(|f| f.size).sum()
}

/// Why a fetch stopped. `Cancelled` is the user's doing and keeps the partial
/// file; `Failed` carries the whole chain, since the one thing the settings
/// pane needs to show is the actual reason.
#[derive(Debug)]
pub(crate) enum FetchError {
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("下载已取消"),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

fn failed(context: impl std::fmt::Display, error: impl std::fmt::Display) -> FetchError {
    FetchError::Failed(format!("{context}: {error}"))
}

/// Live byte accounting, shared between the loader thread and `progress()`.
#[derive(Default)]
pub(crate) struct Counters {
    /// Bytes of files that passed their check.
    verified: AtomicU64,
    /// Bytes written so far to the file in flight, prefix included.
    inflight: AtomicU64,
    file: Mutex<Option<String>>,
}

impl Counters {
    /// `static`-friendly: the loader and the settings poll share one instance.
    pub const fn new() -> Self {
        Self {
            verified: AtomicU64::new(0),
            inflight: AtomicU64::new(0),
            file: Mutex::new(None),
        }
    }

    /// What the bar shows: settled files plus what has landed of the current
    /// one. Never exceeds [`total_bytes`].
    pub fn bytes(&self) -> u64 {
        self.verified.load(Ordering::Relaxed) + self.inflight.load(Ordering::Relaxed)
    }

    /// The file being fetched — the dialog's "正在下载 …".
    pub fn file(&self) -> Option<String> {
        self.file.lock().ok().and_then(|f| f.clone())
    }

    fn begin(&self, name: &str) {
        self.inflight.store(0, Ordering::Relaxed);
        if let Ok(mut slot) = self.file.lock() {
            *slot = Some(name.to_string());
        }
    }

    /// Start a fresh pass. A retry walks the same files again, and counting
    /// their sizes on top of what the failed attempt already settled would
    /// push the bar past 100%.
    pub fn reset(&self) {
        self.verified.store(0, Ordering::Relaxed);
        self.inflight.store(0, Ordering::Relaxed);
        if let Ok(mut slot) = self.file.lock() {
            *slot = None;
        }
    }

    fn settle(&self, size: u64) {
        self.inflight.store(0, Ordering::Relaxed);
        self.verified.fetch_add(size, Ordering::Relaxed);
        if let Ok(mut slot) = self.file.lock() {
            *slot = None;
        }
    }
}

/// Make `dir` hold a verified copy of every file in [`FILES`]. Returns them in
/// the same order.
pub(crate) fn ensure(
    dir: &Path,
    counters: &Counters,
    cancel: &AtomicBool,
) -> Result<Vec<PathBuf>, FetchError> {
    ensure_from(&endpoints(), dir, FILES, counters, cancel)
}

/// The endpoint list, most preferred first. `HF_ENDPOINT` names a mirror the
/// user trusts; the public mirror is only a fallback, never a substitute for
/// the canonical host when that one answers.
fn endpoints() -> Vec<String> {
    let custom = std::env::var("HF_ENDPOINT")
        .ok()
        .map(|e| e.trim().trim_end_matches('/').to_string())
        .filter(|e| !e.is_empty());
    let candidates = [
        custom,
        Some(DEFAULT_ENDPOINT.to_string()),
        Some(MIRROR_ENDPOINT.to_string()),
    ];
    let mut out: Vec<String> = Vec::new();
    for candidate in candidates.into_iter().flatten() {
        // A custom endpoint that happens to BE the mirror must not be fetched
        // twice.
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

/// The download core, with the endpoint list and file set passed in so tests
/// can drive it against a local server and a handful of bytes.
pub(crate) fn ensure_from(
    endpoints: &[String],
    dir: &Path,
    files: &[FileSpec],
    counters: &Counters,
    cancel: &AtomicBool,
) -> Result<Vec<PathBuf>, FetchError> {
    counters.reset();
    let repo_dir = dir.join(REPO);
    std::fs::create_dir_all(&repo_dir).map_err(|e| failed("无法创建模型目录", e))?;
    let mut out = Vec::with_capacity(files.len());
    for spec in files {
        let path = repo_dir.join(spec.name);
        counters.begin(spec.name);
        if !verified(&path, spec) {
            adopt_legacy(dir, spec, &path);
        }
        if !verified(&path, spec) {
            counters.inflight.store(part_len(&path), Ordering::Relaxed);
            download(endpoints, spec, &path, counters, cancel)?;
        }
        counters.settle(spec.size);
        out.push(path);
    }
    Ok(out)
}

fn part_path(final_path: &Path) -> PathBuf {
    let mut name = final_path.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

fn part_len(final_path: &Path) -> u64 {
    std::fs::metadata(part_path(final_path))
        .map(|m| m.len())
        .unwrap_or(0)
}

/// The stamp a verified file leaves behind: re-hashing the weights on every
/// launch would make each one pay a few hundred milliseconds for nothing.
fn stamp_path(final_path: &Path) -> PathBuf {
    let mut name = final_path.as_os_str().to_os_string();
    name.push(".verified");
    PathBuf::from(name)
}

fn expected_stamp(spec: &FileSpec) -> String {
    format!("{} {}", spec.size, spec.sha256.unwrap_or("-"))
}

/// Whether the bytes at `path` are the file `spec` describes. Always hashes —
/// [`verified`] is the version with the stamp fast path.
fn check(path: &Path, spec: &FileSpec) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.len() != spec.size {
        return false;
    }
    let Ok((size, digest)) = hash_file(path) else {
        return false;
    };
    size == spec.size && spec.sha256.is_none_or(|want| want == digest)
}

/// Whether `path` is a file this module already verified. The stamp saves
/// re-hashing the 235 MB weight on every launch; without it the hash decides.
fn verified(path: &Path, spec: &FileSpec) -> bool {
    let expected = expected_stamp(spec);
    if std::fs::read_to_string(stamp_path(path))
        .map(|s| s.trim() == expected)
        .unwrap_or(false)
    {
        return std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() == spec.size);
    }
    if !check(path, spec) {
        return false;
    }
    let _ = std::fs::write(stamp_path(path), format!("{expected}\n"));
    true
}

/// Whether every model file is already on disk and stamped. This is what the
/// settings pane asks before offering a download: a machine that has the
/// weights must not be asked to fetch them again. It reads the stamps and the
/// file sizes — never the bytes.
pub(crate) fn present(dir: &Path) -> bool {
    let repo = dir.join(REPO);
    FILES.iter().all(|spec| {
        let path = repo.join(spec.name);
        std::fs::read_to_string(stamp_path(&path))
            .map(|stamp| stamp.trim() == expected_stamp(spec))
            .unwrap_or(false)
            && std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() == spec.size)
    })
}

/// Size and sha256 of a file, read in streaming chunks so a 235 MB weight
/// never sits in memory.
fn hash_file(path: &Path) -> std::io::Result<(u64, String)> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok((size, format!("{:x}", hasher.finalize())))
}

/// The old hub client kept verified blobs under `models--<repo>/snapshots/`
/// with a commit hash for a directory name. A user upgrading past that build
/// holds a perfectly good 235 MB weight — copy it instead of fetching again.
///
/// Best effort by design: the snapshot entries are symlinks into `blobs/`, a
/// relocated or half-deleted cache has broken ones, and none of that is worth
/// failing the fetch over. A copy (not a link) because a hard link to the
/// symlink itself would carry its relative target along and resolve to
/// nothing at the new path.
fn adopt_legacy(dir: &Path, spec: &FileSpec, target: &Path) {
    let legacy_root = dir.join(format!("models--{}", REPO.replace('/', "--")));
    let Ok(snapshots) = std::fs::read_dir(legacy_root.join("snapshots")) else {
        return;
    };
    for entry in snapshots.flatten() {
        let candidate = entry.path().join(spec.name);
        if !check(&candidate, spec) {
            continue;
        }
        match std::fs::copy(&candidate, target) {
            Ok(_) => {
                let _ = std::fs::write(stamp_path(target), format!("{}\n", expected_stamp(spec)));
                tracing::info!(file = spec.name, "reused a verified model file from the old cache");
                return;
            }
            // Whatever went wrong here, the download below is the answer.
            Err(error) => {
                tracing::debug!(%error, file = spec.name, "old cache copy failed; downloading");
            }
        }
    }
}

/// Attempts per route per endpoint. With the direct fallback in [`routes`] the
/// whole budget is two routes × two endpoints × two attempts.
const ATTEMPTS: u32 = 2;
const CHUNK: usize = 64 * 1024;

/// One way out to the hub, with the name the log uses.
struct Route {
    label: &'static str,
    agent: ureq::Agent,
}

/// Every way this host can reach the hub, most likely first: the proxy the
/// environment or the system names, then a direct connection. A proxy left
/// configured but no longer running refuses the connection instantly, and
/// before this fallback existed that refusal was the entire answer.
fn routes() -> Result<Vec<Route>, FetchError> {
    let mut out = Vec::new();
    if let Some(proxy) = proxy() {
        out.push(Route {
            label: "proxy",
            agent: build(Some(proxy))?,
        });
    }
    out.push(Route {
        label: "direct",
        agent: build(None)?,
    });
    Ok(out)
}

/// Fetch one file to `final_path`, resuming a `.part` when the server honours
/// `Range` and starting over when it does not. Every route and endpoint is
/// tried before the budget runs out; a file only takes its final name after it
/// matches `spec`, so a half-transfer can never be loaded.
fn download(
    endpoints: &[String],
    spec: &FileSpec,
    final_path: &Path,
    counters: &Counters,
    cancel: &AtomicBool,
) -> Result<(), FetchError> {
    let routes = routes()?;
    let part = part_path(final_path);
    let mut last: Option<FetchError> = None;
    for route in &routes {
        for endpoint in endpoints {
            for attempt in 0..ATTEMPTS {
                if cancel.load(Ordering::Relaxed) {
                    return Err(FetchError::Cancelled);
                }
                if attempt > 0 {
                    std::thread::sleep(Duration::from_millis(400 * u64::from(attempt)));
                }
                match transfer(&route.agent, endpoint, spec, &part, counters, cancel) {
                    Ok(()) => return finalise(spec, &part, final_path),
                    Err(FetchError::Cancelled) => return Err(FetchError::Cancelled),
                    Err(error) => {
                        tracing::warn!(
                            file = spec.name,
                            route = route.label,
                            endpoint = %endpoint,
                            attempt,
                            %error,
                            "model file fetch failed"
                        );
                        last = Some(error);
                    }
                }
            }
        }
        tracing::info!(file = spec.name, route = route.label, "model fetch route exhausted");
    }
    // Name what was tried: "connection refused" alone reads as "the app is
    // broken", while "tried proxy / direct" points at the proxy.
    let tried = routes
        .iter()
        .map(|route| route.label)
        .collect::<Vec<_>>()
        .join(" / ");
    Err(match last {
        Some(FetchError::Failed(message)) => {
            FetchError::Failed(format!("{message}（已尝试 {tried}）"))
        }
        _ => FetchError::Failed(format!("{} 无法下载（已尝试 {tried}）", spec.name)),
    })
}

/// One request. Returns Ok only when the bytes on disk add up to `spec.size`
/// AND hash to `spec.sha256`; the caller deletes the part file otherwise.
fn transfer(
    agent: &ureq::Agent,
    endpoint: &str,
    spec: &FileSpec,
    part: &Path,
    counters: &Counters,
    cancel: &AtomicBool,
) -> Result<(), FetchError> {
    let url = format!("{endpoint}/{REPO}/resolve/{REVISION}/{}", spec.name);
    let have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    // A part at or past the expected length is not resumable: whatever is in
    // it is wrong.
    let have = if have > spec.size { 0 } else { have };
    let mut hasher = Sha256::new();
    if have > 0 {
        // The hash must cover the bytes already on disk too, or a resumed
        // download would verify against a prefix it never read.
        let mut prefix = std::fs::File::open(part).map_err(|e| failed("无法读取续传文件", e))?;
        let mut buf = vec![0u8; 1024 * 1024];
        let mut seen = 0u64;
        while seen < have {
            let want = (have - seen).min(CHUNK as u64) as usize;
            let n = prefix.read(&mut buf[..want]).map_err(|e| failed("无法读取续传文件", e))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            seen += n as u64;
        }
        if seen != have {
            let _ = std::fs::remove_file(part);
            return Err(FetchError::Failed("续传文件长度与预期不符，已丢弃".into()));
        }
    }

    let mut request = agent.get(&url);
    if have > 0 {
        request = request.header("Range", &format!("bytes={have}-"));
    }
    let response = request
        .call()
        .map_err(|e| failed(format!("请求 {url} 失败"), e))?;

    // 206 continues the part file; 200 means the server ignored the range and
    // is sending the whole thing, so the part must be discarded first.
    let mut keep = have;
    match response.status().as_u16() {
        206 => {}
        200 => {
            hasher = Sha256::new();
            keep = 0;
        }
        416 => {
            let _ = std::fs::remove_file(part);
            return Err(FetchError::Failed("服务器拒绝续传范围".into()));
        }
        code => {
            return Err(FetchError::Failed(format!(
                "请求 {url} 返回 HTTP {code}"
            )))
        }
    }

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(part)
        .map_err(|e| failed("无法写入模型缓存", e))?;
    file.set_len(keep).map_err(|e| failed("无法写入模型缓存", e))?;
    file.seek(SeekFrom::End(0))
        .map_err(|e| failed("无法写入模型缓存", e))?;

    let mut body = response.into_body().into_reader();
    let mut buf = vec![0u8; CHUNK];
    let mut written = keep;
    counters.inflight.store(written, Ordering::Relaxed);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(FetchError::Cancelled);
        }
        let n = body.read(&mut buf).map_err(|e| failed("下载中断", e))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| failed("无法写入模型缓存", e))?;
        hasher.update(&buf[..n]);
        written += n as u64;
        counters.inflight.store(written, Ordering::Relaxed);
        if written > spec.size {
            let _ = std::fs::remove_file(part);
            return Err(FetchError::Failed(format!(
                "{} 超出预期大小，已丢弃",
                spec.name
            )));
        }
    }
    file.flush().map_err(|e| failed("无法写入模型缓存", e))?;

    if written != spec.size {
        return Err(FetchError::Failed(format!(
            "{} 只下到 {written} / {} 字节",
            spec.name, spec.size
        )));
    }
    let digest = format!("{:x}", hasher.finalize());
    if let Some(want) = spec.sha256 {
        if digest != want {
            let _ = std::fs::remove_file(part);
            return Err(FetchError::Failed(format!(
                "{} 校验失败，已删除缓存文件",
                spec.name
            )));
        }
    }
    Ok(())
}

/// Promote the finished part file. The rename is what makes the file loadable,
/// so a crash mid-download leaves nothing the loader would accept.
fn finalise(spec: &FileSpec, part: &Path, final_path: &Path) -> Result<(), FetchError> {
    if let Ok(file) = std::fs::File::open(part) {
        let _ = file.sync_all();
    }
    std::fs::rename(part, final_path).map_err(|e| failed("无法写入模型缓存", e))?;
    std::fs::write(stamp_path(final_path), format!("{}\n", expected_stamp(spec)))
        .map_err(|e| failed("无法写入模型缓存", e))?;
    Ok(())
}

/// A client for one route: system roots (a corporate CA or a local packet
/// inspector is trusted, because the OS trusts it) and a timeout on every
/// phase, so a stalled transfer eventually fails instead of hanging the loader
/// thread forever.
fn build(proxy: Option<ureq::Proxy>) -> Result<ureq::Agent, FetchError> {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    let config = ureq::Agent::config_builder()
        .tls_config(tls)
        .proxy(proxy)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(60)))
        .http_status_as_error(false)
        .max_redirects(10)
        .build();
    Ok(ureq::Agent::new_with_config(config))
}

/// Proxy for the fetch. `ureq` reads none of these itself in the hub client
/// this module replaces, so the whole reason a Mac sat behind ClashX and never
/// downloaded is fixed here.
fn proxy() -> Option<ureq::Proxy> {
    if let Some(proxy) = ureq::Proxy::try_from_env() {
        return Some(proxy);
    }
    system_proxy()
}

#[cfg(target_os = "macos")]
fn system_proxy() -> Option<ureq::Proxy> {
    // `scutil` is the only way to read the network service's HTTP proxy without
    // pulling in the SystemConfiguration bindings. A disabled or absent block
    // simply yields nothing.
    let output = std::process::Command::new("scutil")
        .arg("--proxy")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let field = |key: &str| {
        text.lines()
            .find_map(|line| line.trim().strip_prefix(key))
            .and_then(|rest| rest.trim().strip_prefix(':'))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    for (enable, host, port) in [
        ("HTTPSEnable", "HTTPSProxy", "HTTPSPort"),
        ("HTTPEnable", "HTTPProxy", "HTTPPort"),
    ] {
        if field(enable).as_deref() != Some("1") {
            continue;
        }
        let (Some(host), Some(port)) = (field(host), field(port)) else {
            continue;
        };
        if let Ok(proxy) = ureq::Proxy::new(&format!("http://{host}:{port}")) {
            return Some(proxy);
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn system_proxy() -> Option<ureq::Proxy> {
    None
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use super::*;

    fn sha(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    /// Range starts seen by the test server, one per request.
    type RangeLog = Arc<Mutex<Vec<Option<usize>>>>;

    /// A one-shot HTTP file server. `honour_range` decides whether it answers a
    /// `Range` request with 206 or ignores it with 200 — the two cases the
    /// resume path has to handle. Every request's range start is recorded so a
    /// test can prove a transfer actually resumed.
    fn serve(
        body: Vec<u8>,
        honour_range: bool,
        requests: usize,
    ) -> (String, RangeLog, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let handle = std::thread::spawn(move || {
            for _ in 0..requests {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut start = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                        break;
                    }
                    if line.to_ascii_lowercase().starts_with("range:") {
                        start = line
                            .split('=')
                            .nth(1)
                            .and_then(|v| v.trim().trim_end_matches('-').parse::<usize>().ok());
                    }
                }
                log.lock().unwrap().push(start);
                let (status, from) = match (honour_range, start) {
                    (true, Some(n)) if n > 0 => ("206 Partial Content", n.min(body.len())),
                    _ => ("200 OK", 0),
                };
                let slice = &body[from..];
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    slice.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(slice);
                let _ = stream.flush();
            }
        });
        (endpoint, seen, handle)
    }

    /// A spec whose digest outlives the fixture it was computed from.
    fn checked(name: &'static str, body: &[u8]) -> FileSpec {
        FileSpec {
            name,
            size: body.len() as u64,
            sha256: Some(Box::leak(sha(body).into_boxed_str())),
        }
    }

    fn unchecked(name: &'static str, body: &[u8]) -> FileSpec {
        FileSpec {
            name,
            size: body.len() as u64,
            sha256: None,
        }
    }

    /// The happy path: bytes arrive, they match, and the file takes its final
    /// name — nothing loadable is left behind before that.
    #[test]
    fn a_matching_download_is_promoted() {
        let body = b"hello husk model".to_vec();
        let spec = checked("model.bin", &body);
        let (endpoint, _seen, server) = serve(body.clone(), true, 1);
        let dir = tempfile::tempdir().unwrap();
        let counters = Counters::new();

        let paths = ensure_from(
            &[endpoint],
            dir.path(),
            std::slice::from_ref(&spec),
            &counters,
            &AtomicBool::new(false),
        )
        .unwrap();

        assert_eq!(std::fs::read(&paths[0]).unwrap(), body);
        assert!(!part_path(&paths[0]).exists());
        assert_eq!(counters.bytes(), body.len() as u64);
        assert_eq!(counters.file(), None);
        server.join().unwrap();
    }

    /// A body that does not hash right is discarded, not kept: the whole point
    /// of the check is that a bad file can never become loadable.
    #[test]
    fn a_mismatched_body_is_deleted() {
        let body = b"corrupt".to_vec();
        let spec = FileSpec {
            name: "model.bin",
            size: body.len() as u64,
            sha256: Some("0000000000000000000000000000000000000000000000000000000000000000"),
        };
        let (endpoint, _seen, server) = serve(body, true, 1);
        let dir = tempfile::tempdir().unwrap();
        let counters = Counters::new();

        let error = ensure_from(
            &[endpoint],
            dir.path(),
            std::slice::from_ref(&spec),
            &counters,
            &AtomicBool::new(false),
        )
        .unwrap_err();

        assert!(matches!(error, FetchError::Failed(_)), "{error}");
        let final_path = dir.path().join(REPO).join("model.bin");
        assert!(!final_path.exists());
        assert!(!part_path(&final_path).exists());
        server.join().unwrap();
    }

    /// A `.part` left by an interrupted run is completed with a `Range`
    /// request, not re-downloaded from zero.
    #[test]
    fn a_partial_file_resumes() {
        let body = b"0123456789abcdef".to_vec();
        let spec = checked("model.bin", &body);
        let (endpoint, seen, server) = serve(body.clone(), true, 1);
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join(REPO).join("model.bin");
        std::fs::create_dir_all(final_path.parent().unwrap()).unwrap();
        std::fs::write(part_path(&final_path), &body[..8]).unwrap();

        ensure_from(
            &[endpoint],
            dir.path(),
            std::slice::from_ref(&spec),
            &Counters::new(),
            &AtomicBool::new(false),
        )
        .unwrap();

        assert_eq!(std::fs::read(&final_path).unwrap(), body);
        assert_eq!(*seen.lock().unwrap(), vec![Some(8)]);
        server.join().unwrap();
    }

    /// A server that ignores `Range` answers 200 with the whole body — the
    /// partial file must be thrown away rather than appended to, which is
    /// exactly how the old client corrupted its cache.
    #[test]
    fn a_server_ignoring_range_restarts_the_file() {
        let body = b"0123456789abcdef".to_vec();
        let spec = checked("model.bin", &body);
        let (endpoint, _seen, server) = serve(body.clone(), false, 1);
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join(REPO).join("model.bin");
        std::fs::create_dir_all(final_path.parent().unwrap()).unwrap();
        std::fs::write(part_path(&final_path), b"stale-bytes").unwrap();

        ensure_from(
            &[endpoint],
            dir.path(),
            std::slice::from_ref(&spec),
            &Counters::new(),
            &AtomicBool::new(false),
        )
        .unwrap();

        assert_eq!(std::fs::read(&final_path).unwrap(), body);
        server.join().unwrap();
    }

    /// Cancelling stops the transfer but keeps what already landed — the next
    /// attempt resumes instead of paying for the bytes twice.
    #[test]
    fn cancelling_keeps_the_part_file() {
        let body = b"0123456789abcdef".to_vec();
        let spec = unchecked("model.bin", &body);
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join(REPO).join("model.bin");
        std::fs::create_dir_all(final_path.parent().unwrap()).unwrap();
        std::fs::write(part_path(&final_path), &body[..4]).unwrap();

        let error = ensure_from(
            &["http://127.0.0.1:1".to_string()],
            dir.path(),
            std::slice::from_ref(&spec),
            &Counters::new(),
            &AtomicBool::new(true),
        )
        .unwrap_err();

        assert!(matches!(error, FetchError::Cancelled), "{error}");
        assert_eq!(std::fs::read(part_path(&final_path)).unwrap(), &body[..4]);
        assert!(!final_path.exists());
    }

    /// A cache written by the old hub client is reused when it verifies — the
    /// endpoint here is unreachable, so only adoption can make this pass.
    #[test]
    fn a_verified_legacy_copy_is_adopted() {
        let body = b"already on disk".to_vec();
        let spec = checked("model.bin", &body);
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir
            .path()
            .join(format!("models--{}", REPO.replace('/', "--")))
            .join("snapshots")
            .join("deadbeef");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("model.bin"), &body).unwrap();

        let paths = ensure_from(
            &["http://127.0.0.1:1".to_string()],
            dir.path(),
            std::slice::from_ref(&spec),
            &Counters::new(),
            &AtomicBool::new(false),
        )
        .unwrap();

        assert_eq!(std::fs::read(&paths[0]).unwrap(), body);
    }

    /// The bar's denominator is the manifest, not a guess.
    #[test]
    fn the_total_is_the_manifest_sum() {
        assert_eq!(total_bytes(), 252_138_706);
    }

    /// The real hub, for the files that are small enough to fetch in a test
    /// run. Ignored by default: it needs the network.
    #[test]
    #[ignore]
    fn zz_live_fetch_small_files() {
        let small: Vec<FileSpec> = FILES
            .iter()
            .filter(|f| f.size < 2048)
            .map(|f| FileSpec {
                name: f.name,
                size: f.size,
                sha256: f.sha256,
            })
            .collect();
        assert_eq!(small.len(), 3);
        let dir = tempfile::tempdir().unwrap();
        let counters = Counters::new();
        let paths = ensure_from(
            &endpoints(),
            dir.path(),
            &small,
            &counters,
            &AtomicBool::new(false),
        )
        .expect("live fetch");
        for (path, spec) in paths.iter().zip(&small) {
            assert_eq!(std::fs::metadata(path).unwrap().len(), spec.size);
        }
        eprintln!("live fetch ok: {} bytes total", counters.bytes());
    }
}
