//! `linux_bwrap.rs` — bubblewrap backend (primary on Linux).
//!
//! Sandboxing model:
//! 1. Base system libraries and binaries (/usr, /lib, /bin, /lib64, /opt, /snap) are mounted read-only.
//! 2. Essential system configuration (/etc/ssl, /etc/pki, /etc/hosts, /etc/passwd, /etc/ld.so.cache,
//!    /etc/alternatives, etc.) is mounted read-only so that dynamic linking, TLS/HTTPS, DNS resolution,
//!    and system alternatives work seamlessly.
//! 3. Process-isolated /tmp (tmpfs) is mounted read-write, with /var/tmp symlinked, enabling temporary
//!    file creation by runtimes (bun, node, python, gcc, etc.).
//! 4. System and user development environments (bun, volta, cargo, rustup, nvm, fnm, deno, pnpm,
//!    python/pyenv, conda, go, sdkman, and tools from PATH) are discovered and mounted read-only.
//! 5. Sensitive credentials and secrets (.ssh, .gnupg, .aws, .cargo/credentials*, .npmrc, etc.)
//!    are masked with /dev/null or omitted entirely.
//! 6. Toolchain cache environment variables (BUN_INSTALL_CACHE_DIR, npm_config_cache, etc.) are
//!    redirected to /tmp to prevent EROFS errors while keeping host caches immutable.
//! 7. The workspace is mounted read-write last, ensuring workspace access takes precedence.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};

use crate::env_sanitize::sanitize_env;
use crate::traits::{CommandOutput, SandboxBackend, SandboxConfig, SandboxTier};

pub struct LinuxBwrap;

/// Base system roots mounted read-only (if they exist on the host).
const SYSTEM_RO_DIRS: &[&str] = &[
    "/usr",
    "/lib",
    "/bin",
    "/lib64",
    "/opt",
    "/snap",
    "/var/lib/snapd/snap",
];

/// Essential system configuration files/dirs mounted read-only (if they exist).
const SYSTEM_CONFIG_PATHS: &[&str] = &[
    // SSL / TLS certificates & PKI (crucial for curl, git, bun, npm, pip, cargo)
    "/etc/ssl",
    "/etc/pki",
    "/etc/ca-certificates",
    "/etc/crypto-policies",
    // Name resolution & user/group databases
    "/etc/hosts",
    "/etc/passwd",
    "/etc/group",
    "/etc/nsswitch.conf",
    // Dynamic linker configuration and alternatives
    "/etc/ld.so.cache",
    "/etc/ld.so.conf",
    "/etc/ld.so.conf.d",
    "/etc/alternatives",
    // Timezone and system profiles
    "/etc/localtime",
    "/etc/zoneinfo",
    "/etc/profile",
    "/etc/profile.d",
    "/etc/environment",
    "/etc/mime.types",
    // System git config
    "/etc/gitconfig",
];

/// Sensitive dirs and files denied outright — masked with /dev/null or never mounted.
const SENSITIVE_NAMES: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".kube",
    ".bash_history",
    ".zsh_history",
    ".history",
    ".config/agent-rs",
    ".config/husk",
    ".cargo/credentials",
    ".cargo/credentials.toml",
    ".npmrc",
    ".netrc",
];

/// Common developer toolchain and runtime directories relative to HOME.
const HOME_DEV_DIRS: &[&str] = &[
    ".cargo",
    ".rustup",
    ".volta",
    ".bun",
    ".nvm",
    ".fnm",
    ".deno",
    ".pnpm",
    ".yarn",
    ".config/yarn",
    ".npm",
    ".cache",
    ".pyenv",
    "miniconda3",
    "anaconda3",
    ".conda",
    ".rye",
    ".local/share/uv",
    ".local/pipx",
    "go",
    ".config/go",
    ".sdkman",
    ".gradle",
    ".m2",
    ".asdf",
    ".mise",
    ".local/share/mise",
    ".config/mise",
    // Common user binaries and libraries
    ".local/bin",
    ".local/lib",
    ".local/share/pnpm",
    ".local/share/fnm",
];

/// Toolchain root environment variables to check for custom installation paths.
const ENV_TOOLCHAIN_VARS: &[&str] = &[
    "VOLTA_HOME",
    "BUN_INSTALL",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "NVM_DIR",
    "FNM_DIR",
    "PNPM_HOME",
    "DENO_INSTALL",
    "PYENV_ROOT",
    "GOROOT",
    "GOPATH",
    "JAVA_HOME",
    "ANDROID_HOME",
    "ANDROID_SDK_ROOT",
    "FLUTTER_ROOT",
];

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Check if a candidate path is sensitive or contains sensitive data.
fn is_sensitive(path: &Path, home: Option<&Path>) -> bool {
    let s = path.to_string_lossy();
    for name in SENSITIVE_NAMES {
        if s.ends_with(name) || s.contains(&format!("/{name}/")) || s.contains(&format!("/{name}")) {
            return true;
        }
    }
    if let Some(h) = home {
        if let Ok(rel) = path.strip_prefix(h) {
            let rel_str = rel.to_string_lossy();
            for name in SENSITIVE_NAMES {
                if rel_str == *name || rel_str.starts_with(&format!("{name}/")) {
                    return true;
                }
            }
        }
    }
    false
}

/// Check if a path is within the base system mounts (already mounted).
fn is_in_base_system(p: &Path) -> bool {
    SYSTEM_RO_DIRS.iter().any(|sys| p.starts_with(sys))
}

/// Deduplicate mount paths so that parent mounts supersede child mounts,
/// and HOME itself is never mounted wholesale.
fn deduplicate_mount_paths(paths: Vec<PathBuf>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut sorted = paths;
    sorted.sort_by_key(|p| p.as_os_str().len());
    sorted.dedup();

    let mut clean: Vec<PathBuf> = Vec::new();
    for p in sorted {
        if let Some(h) = home {
            if &p == h {
                continue; // Never mount entire HOME
            }
        }
        let already_covered = clean.iter().any(|existing| p.starts_with(existing));
        if !already_covered {
            clean.push(p);
        }
    }
    clean
}

/// Discover host development environments, toolchains, and runtimes.
fn dev_environment_binds() -> Vec<PathBuf> {
    let mut mounts = Vec::new();
    let home = dirs_home();

    // 1. Check well-known toolchain directories in HOME
    if let Some(ref h) = home {
        for rel in HOME_DEV_DIRS {
            let p = h.join(rel);
            if p.exists() && !is_sensitive(&p, home.as_deref()) {
                mounts.push(p);
            }
        }
        // Also mount ~/.gitconfig if present so git operations know author identity
        let gitconfig = h.join(".gitconfig");
        if gitconfig.is_file() {
            mounts.push(gitconfig);
        }
    }

    // 2. Check explicit toolchain environment variables
    for var in ENV_TOOLCHAIN_VARS {
        if let Some(val) = std::env::var_os(var) {
            let p = PathBuf::from(val);
            if p.exists() && !is_sensitive(&p, home.as_deref()) {
                if let Some(ref h) = home {
                    if &p == h {
                        continue;
                    }
                }
                mounts.push(p);
            }
        }
    }

    // 3. Check directories in host PATH
    if let Some(path_var) = std::env::var_os("PATH") {
        for entry in std::env::split_paths(&path_var) {
            if entry.is_dir() && !is_sensitive(&entry, home.as_deref()) {
                if let Some(ref h) = home {
                    if &entry == h {
                        continue;
                    }
                }
                if is_in_base_system(&entry) {
                    continue;
                }
                // If entry is `<parent>/bin`, mount the `<parent>` toolchain root (e.g. Flutter SDK)
                if entry.file_name().map_or(false, |n| n == "bin") {
                    if let Some(parent) = entry.parent() {
                        if Some(parent) != home.as_deref() && parent.is_dir() {
                            mounts.push(parent.to_path_buf());
                            continue;
                        }
                    }
                }
                mounts.push(entry);
            }
        }
    }

    deduplicate_mount_paths(mounts, home.as_deref())
}

/// Collect sensitive files/directories that exist on host and must be masked.
fn sensitive_masks() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = dirs_home() {
        for name in SENSITIVE_NAMES {
            let p = home.join(name);
            if p.exists() {
                out.push(p);
            }
        }
    }
    let etc_ssh = PathBuf::from("/etc/ssh");
    if etc_ssh.exists() {
        out.push(etc_ssh);
    }
    out
}

/// Per-run tmp dir — `/run/user/$UID/agent-run-<pid>` (tmpfs-scoped,
/// auto-cleaned on logout). Falls back to `std::env::temp_dir` when the XDG
/// runtime dir is absent.
fn run_dir() -> PathBuf {
    let uid = libc_uid();
    let xdg = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{uid}")));
    let dir = xdg.join(format!("agent-run-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// `getuid` without pulling libc — read `/proc/self/status`'s Uid line.
fn libc_uid() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(1000)
}

#[async_trait::async_trait]
impl SandboxBackend for LinuxBwrap {
    fn id(&self) -> &'static str {
        "bwrap"
    }

    fn tier(&self) -> SandboxTier {
        SandboxTier::L2
    }

    async fn run_command(
        &self,
        cmd: &str,
        _args: &[&str],
        cfg: &SandboxConfig,
    ) -> Result<CommandOutput> {
        let started = Instant::now();
        let run_dir = run_dir();
        let ws = cfg
            .workspace_dir
            .canonicalize()
            .unwrap_or_else(|_| cfg.workspace_dir.clone());

        let mut argv: Vec<String> = Vec::with_capacity(128);

        // ---- 1. ro base system binds ----
        for sys in SYSTEM_RO_DIRS {
            if Path::new(sys).exists() {
                argv.push("--ro-bind".into());
                argv.push((*sys).into());
                argv.push((*sys).into());
            }
        }

        // ---- 2. ro system configuration (/etc/ssl, /etc/pki, /etc/hosts, /etc/alternatives, etc.) ----
        for conf in SYSTEM_CONFIG_PATHS {
            if Path::new(conf).exists() {
                argv.push("--ro-bind".into());
                argv.push((*conf).into());
                argv.push((*conf).into());
            }
        }

        // resolv.conf: only when network is allowed
        if cfg.allow_network && Path::new("/etc/resolv.conf").exists() {
            argv.push("--ro-bind".into());
            argv.push("/etc/resolv.conf".into());
            argv.push("/etc/resolv.conf".into());
        }

        // ---- 3. special filesystems ----
        argv.push("--proc".into());
        argv.push("/proc".into());
        argv.push("--dev".into());
        argv.push("/dev".into());

        // ---- 4. process-isolated /tmp (tmpfs) and /var/tmp symlink ----
        argv.push("--tmpfs".into());
        argv.push("/tmp".into());
        if Path::new("/var").exists() {
            argv.push("--dir".into());
            argv.push("/var".into());
            argv.push("--symlink".into());
            argv.push("/tmp".into());
            argv.push("/var/tmp".into());
        }

        // Bind run_dir at its host path for XDG_RUNTIME_DIR compatibility
        argv.push("--bind".into());
        argv.push(run_dir.to_string_lossy().into_owned());
        argv.push(run_dir.to_string_lossy().into_owned());

        // ---- 5. developer environments & toolchains (bun, volta, cargo, rustup, python, etc.) ----
        let dev_binds = dev_environment_binds();
        for dev_dir in &dev_binds {
            if dev_dir.exists() {
                argv.push("--ro-bind".into());
                argv.push(dev_dir.to_string_lossy().into_owned());
                argv.push(dev_dir.to_string_lossy().into_owned());
            }
        }

        // ---- 6. caller-specified extra ro mounts ----
        for extra_ro in &cfg.extra_ro_mounts {
            if extra_ro.exists() {
                argv.push("--ro-bind".into());
                argv.push(extra_ro.to_string_lossy().into_owned());
                argv.push(extra_ro.to_string_lossy().into_owned());
            }
        }

        // ---- 7. sensitive dirs & files masked with /dev/null ----
        for mask in sensitive_masks() {
            argv.push("--ro-bind".into());
            argv.push("/dev/null".into());
            argv.push(mask.to_string_lossy().into_owned());
        }

        // ---- 8. caller-specified extra rw mounts ----
        for extra_rw in &cfg.extra_rw_mounts {
            if extra_rw.exists() {
                argv.push("--bind".into());
                argv.push(extra_rw.to_string_lossy().into_owned());
                argv.push(extra_rw.to_string_lossy().into_owned());
            }
        }

        // ---- 9. workspace bind LAST (rw; CoW snapshot mounts at same path) ----
        argv.push("--bind".into());
        argv.push(ws.to_string_lossy().into_owned());
        argv.push(ws.to_string_lossy().into_owned());

        // ---- 10. network isolation ----
        if !cfg.allow_network {
            argv.push("--unshare-net".into());
        }

        // ---- 11. PID namespace: fork-bomb containment ----
        argv.push("--unshare-pid".into());

        // ---- 12. env: clearenv + sanitized set + toolchain cache redirects ----
        argv.push("--clearenv".into());
        let mut extra = cfg.env_vars.clone();
        if !extra.iter().any(|(k, _)| k == "TMPDIR") {
            extra.push(("TMPDIR".into(), "/tmp".into()));
        }
        if !extra.iter().any(|(k, _)| k == "TEMP") {
            extra.push(("TEMP".into(), "/tmp".into()));
        }
        if !extra.iter().any(|(k, _)| k == "TMP") {
            extra.push(("TMP".into(), "/tmp".into()));
        }
        if !extra.iter().any(|(k, _)| k == "CARGO_TARGET_DIR") {
            extra.push(("CARGO_TARGET_DIR".into(), ws.join("target").to_string_lossy().into_owned()));
        }
        // Redirect package manager caches to writable /tmp so installs don't fail with EROFS
        if !extra.iter().any(|(k, _)| k == "BUN_INSTALL_CACHE_DIR") {
            extra.push(("BUN_INSTALL_CACHE_DIR".into(), "/tmp/bun-cache".into()));
        }
        if !extra.iter().any(|(k, _)| k == "npm_config_cache") {
            extra.push(("npm_config_cache".into(), "/tmp/npm-cache".into()));
        }
        if !extra.iter().any(|(k, _)| k == "YARN_CACHE_FOLDER") {
            extra.push(("YARN_CACHE_FOLDER".into(), "/tmp/yarn-cache".into()));
        }

        let mut envs = sanitize_env(&extra);
        // PATH coherence: keep only entries the namespace can resolve —
        // an inherited entry under an unmounted dir is dead weight inside
        // and only widens the executable-hijack surface. Allowed roots =
        // the same dirs just bound (system + dev); workspace and the
        // sandbox tmpfs are denied outright (agent-writable).
        let mut path_roots: Vec<PathBuf> =
            SYSTEM_RO_DIRS.iter().map(PathBuf::from).collect();
        path_roots.extend(dev_binds.iter().cloned());
        let path_denied = vec![
            ws.clone(),
            PathBuf::from("/tmp"),
            run_dir.clone(),
        ];
        crate::env_sanitize::apply_environment_policy(
            &mut envs,
            &cfg.environment,
            Some(&path_roots),
            &path_denied,
        );
        for (k, v) in envs {
            argv.push("--setenv".into());
            argv.push(k);
            argv.push(v);
        }

        argv.push("--chdir".into());
        argv.push(ws.to_string_lossy().into_owned());
        argv.push("--die-with-parent".into());
        argv.push("--".into());
        // Absolute path to sh
        argv.push("/bin/sh".into());
        argv.push("-c".into());

        // Prepend resource limits to the command
        let mut inner = String::new();
        if cfg.max_memory_mb > 0 {
            inner.push_str(&format!("ulimit -v {}; ", cfg.max_memory_mb * 1024));
        }
        if cfg.max_processes > 0 {
            inner.push_str(&format!("ulimit -u {}; ", cfg.max_processes));
        }
        inner.push_str(cmd);
        argv.push(inner);

        // ---- spawn with timeout + tree kill ----
        let timeout = std::time::Duration::from_secs(cfg.timeout_secs.min(600));
        let child = tokio::process::Command::new("bwrap")
            .args(&argv)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("spawn bwrap")?;

        let status = tokio::time::timeout(timeout, child.wait_with_output()).await;
        let elapsed = started.elapsed().as_millis() as u64;

        let (out, err, code, is_timeout) = match status {
            Ok(Ok(o)) => (
                String::from_utf8_lossy(&o.stdout).into_owned(),
                String::from_utf8_lossy(&o.stderr).into_owned(),
                o.status.code().unwrap_or(-1),
                false,
            ),
            Ok(Err(e)) => (String::new(), e.to_string(), -1, false),
            Err(_) => (
                String::new(),
                "[timeout — process tree killed]".to_string(),
                -1,
                true,
            ),
        };

        let _ = std::fs::remove_dir_all(&run_dir);
        Ok(CommandOutput {
            status: code,
            stdout: out,
            stderr: err,
            is_timeout,
            peak_memory_mb: None,
            elapsed_ms: elapsed,
        })
    }
}
