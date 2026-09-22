//! `linux_bwrap.rs` — bubblewrap backend (primary on Linux).
//!
//! 1. System libraries, binaries and essential config (`/usr`, `/lib`, `/bin`,
//!    `/etc/ssl`, `/etc/passwd`, `/etc/ld.so.cache`, …) are mounted read-only so
//!    linking, TLS and DNS resolution work.
//! 2. A tmpfs `/tmp` (with `/var/tmp` symlinked) is writable, plus a per-run dir
//!    for `XDG_RUNTIME_DIR`.
//! 3. Discovered toolchains (bun, volta, cargo, rustup, python, go, …) mount
//!    read-only, and their cache env vars point at `/tmp` so host caches stay
//!    immutable.
//! 4. Credentials (`.ssh`, `.gnupg`, `.aws`, `.npmrc`, agent auth/session state,
//!    …) are masked with `/dev/null`: the content is withheld, not the name.
//! 5. `extra_ro_mounts` — callers pass the user-level skill roots — mount
//!    read-only. A PATH `<root>/bin` expands to `<root>` only for real SDK roots.
//! 6. The workspace mounts read-write last, so it wins any overlap.


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
    // Agent-harness state: credentials (and, for pi, every session transcript)
    // live next to the skills these sandboxes deliberately expose, so they are
    // masked by name even when a discovery rule would have pulled the root in.
    ".pi/agent/auth.json",
    ".pi/agent/.env",
    ".pi/agent/sessions",
    ".claude/.credentials.json",
    ".codex/auth.json",
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

    if let Some(ref h) = home {
        for rel in HOME_DEV_DIRS {
            let p = h.join(rel);
            if p.exists() && !is_sensitive(&p, home.as_deref()) {
                mounts.push(p);
            }
        }
        // ~/.gitconfig lets git know author identity inside the sandbox.
        let gitconfig = h.join(".gitconfig");
        if gitconfig.is_file() {
            mounts.push(gitconfig);
        }
    }

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
                mounts.extend(bin_entry_mounts(&entry, home.as_deref()));
            }
        }
    }

    deduplicate_mount_paths(mounts, home.as_deref())
}

/// True when `p` sits under a hidden entry inside HOME — `~/.pi/agent`,
/// `~/.cargo`, `~/.config/mise`. Such a subtree is either already mounted
/// explicitly ([`HOME_DEV_DIRS`]/[`ENV_TOOLCHAIN_VARS`]) or agent/tool *state*
/// that must be opened by name, never dragged in wholesale by the PATH rule.
fn is_hidden_home_subtree(p: &Path, home: Option<&Path>) -> bool {
    let Some(home) = home else { return false };
    let Ok(rel) = p.strip_prefix(home) else { return false };
    rel.components()
        .next()
        .is_some_and(|c| c.as_os_str().to_string_lossy().starts_with('.'))
}

/// One PATH entry → the directories to mount for it.
///
/// A `<root>/bin` entry normally expands to the whole `<root>`: that is how SDK
/// roots register (Flutter, Android, a version manager's toolchain). Two
/// exceptions, both falling back to the `bin` dir alone: HOME itself, and roots
/// inside a hidden HOME entry. Without the latter, `~/.pi/agent/bin` on PATH
/// mounted the entire agent state directory read-only — `auth.json`, `.env` and
/// every session transcript included.
fn bin_entry_mounts(entry: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    if entry.file_name().is_some_and(|n| n == "bin") {
        if let Some(parent) = entry.parent() {
            if parent.is_dir() && Some(parent) != home && !is_hidden_home_subtree(parent, home) {
                return vec![parent.to_path_buf()];
            }
        }
    }
    vec![entry.to_path_buf()]
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

        for sys in SYSTEM_RO_DIRS {
            if Path::new(sys).exists() {
                argv.push("--ro-bind".into());
                argv.push((*sys).into());
                argv.push((*sys).into());
            }
        }

        for conf in SYSTEM_CONFIG_PATHS {
            if Path::new(conf).exists() {
                argv.push("--ro-bind".into());
                argv.push((*conf).into());
                argv.push((*conf).into());
            }
        }

        // resolv.conf only when network is allowed — it leaks the resolver.
        if cfg.allow_network && Path::new("/etc/resolv.conf").exists() {
            argv.push("--ro-bind".into());
            argv.push("/etc/resolv.conf".into());
            argv.push("/etc/resolv.conf".into());
        }

        argv.push("--proc".into());
        argv.push("/proc".into());
        argv.push("--dev".into());
        argv.push("/dev".into());

        argv.push("--tmpfs".into());
        argv.push("/tmp".into());
        if Path::new("/var").exists() {
            argv.push("--dir".into());
            argv.push("/var".into());
            argv.push("--symlink".into());
            argv.push("/tmp".into());
            argv.push("/var/tmp".into());
        }

        // run_dir bound at its host path for XDG_RUNTIME_DIR compatibility.
        argv.push("--bind".into());
        argv.push(run_dir.to_string_lossy().into_owned());
        argv.push(run_dir.to_string_lossy().into_owned());

        let dev_binds = dev_environment_binds();
        for dev_dir in &dev_binds {
            if dev_dir.exists() {
                argv.push("--ro-bind".into());
                argv.push(dev_dir.to_string_lossy().into_owned());
                argv.push(dev_dir.to_string_lossy().into_owned());
            }
        }

        for extra_ro in &cfg.extra_ro_mounts {
            if extra_ro.exists() {
                argv.push("--ro-bind".into());
                argv.push(extra_ro.to_string_lossy().into_owned());
                argv.push(extra_ro.to_string_lossy().into_owned());
            }
        }

        for mask in sensitive_masks() {
            argv.push("--ro-bind".into());
            argv.push("/dev/null".into());
            argv.push(mask.to_string_lossy().into_owned());
        }

        for extra_rw in &cfg.extra_rw_mounts {
            if extra_rw.exists() {
                argv.push("--bind".into());
                argv.push(extra_rw.to_string_lossy().into_owned());
                argv.push(extra_rw.to_string_lossy().into_owned());
            }
        }

        // Workspace bind must come last — rw, shadows any overlapping mount.
        argv.push("--bind".into());
        argv.push(ws.to_string_lossy().into_owned());
        argv.push(ws.to_string_lossy().into_owned());

        if !cfg.allow_network {
            argv.push("--unshare-net".into());
        }

        // PID namespace — fork-bomb containment.
        argv.push("--unshare-pid".into());

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
        // Package-manager caches → writable /tmp so installs don't EROFS.
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
        argv.push("/bin/sh".into());
        argv.push("-c".into());

        let mut inner = String::new();
        if cfg.max_memory_mb > 0 {
            inner.push_str(&format!("ulimit -v {}; ", cfg.max_memory_mb * 1024));
        }
        if cfg.max_processes > 0 {
            inner.push_str(&format!("ulimit -u {}; ", cfg.max_processes));
        }
        inner.push_str(cmd);
        argv.push(inner);

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


#[cfg(test)]
mod tests {
    use super::*;

    /// PATH registration must expand to a real SDK root, but never to agent
    /// state: `~/.pi/agent/bin` on PATH used to mount `~/.pi/agent` — auth.json,
    /// .env and session transcripts included.
    #[test]
    fn bin_entries_expand_to_sdk_roots_but_not_hidden_home_state() {
        let base = std::env::temp_dir().join(format!("sbx-bind-{}", std::process::id()));
        let sdk_bin = base.join("Development/flutter/bin");
        let agent_bin = base.join(".pi/agent/bin");
        std::fs::create_dir_all(&sdk_bin).unwrap();
        std::fs::create_dir_all(&agent_bin).unwrap();
        let home = Some(base.as_path());

        assert_eq!(
            bin_entry_mounts(&sdk_bin, home),
            vec![base.join("Development/flutter")]
        );
        assert_eq!(bin_entry_mounts(&agent_bin, home), vec![agent_bin.clone()]);
        // HOME as the parent root is also refused (mount the bin dir only).
        let home_bin = base.join("bin");
        std::fs::create_dir_all(&home_bin).unwrap();
        assert_eq!(bin_entry_mounts(&home_bin, home), vec![home_bin]);

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn hidden_home_subtree_detection() {
        let home = Path::new("/home/u");
        assert!(is_hidden_home_subtree(Path::new("/home/u/.pi/agent"), Some(home)));
        assert!(is_hidden_home_subtree(Path::new("/home/u/.cargo"), Some(home)));
        assert!(!is_hidden_home_subtree(Path::new("/home/u/Development/flutter"), Some(home)));
        assert!(!is_hidden_home_subtree(Path::new("/opt/tool"), Some(home)));
        assert!(!is_hidden_home_subtree(Path::new("/home/u"), None));
    }
}
