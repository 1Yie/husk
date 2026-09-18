//! `linux_bwrap.rs` — bubblewrap backend (primary on Linux).
//!
//! The proven argument set (sandbox-model.md §Platform backends):
//!
//! ```text
//! bwrap
//!   --ro-bind /usr /usr --ro-bind /lib /lib --ro-bind /bin /bin
//!   [--ro-bind /etc/resolv.conf /etc/resolv.conf]   # only when allow_network
//!   --proc /proc --dev /dev
//!   --ro-bind $CARGO_HOME/registry …                # toolchain caches: ro, BEFORE ws
//!   --bind  $RUN_DIR $RUN_DIR                        # per-run tmp
//!   --bind  $EFFECTIVE_WS $WORKSPACE                 # rw: real ws or CoW snapshot
//!   [--unshare-net]                                  # when !allow_network
//!   --clearenv --setenv K V ...
//!   --setenv CARGO_TARGET_DIR $WORKSPACE/target
//!   --setenv TMPDIR $RUN_DIR
//!   --chdir $WORKSPACE
//!   --die-with-parent
//!   -- sh -c "<cmd>"
//! ```
//!
//! **Mount ordering is load-bearing**: read-only toolchain caches mount
//! *before* the workspace bind so a hostile repo can't shadow them; the
//! per-run tmp must never be shadowed by a later mount (it lives at
//! `/run/user/$UID/agent-run-*`, bound as itself — no blanket `--tmpfs /tmp`).

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};

use crate::env_sanitize::sanitize_env;
use crate::traits::{CommandOutput, SandboxBackend, SandboxConfig, SandboxTier};

pub struct LinuxBwrap;

/// Sensitive dirs denied outright — bwrap ro-binds the system but these must
/// not be readable even read-only inside the sandbox.
const SENSITIVE_DIRS: &[&str] = &[
    ".ssh", ".gnupg", ".aws", ".azure", ".kube",
    ".bash_history", ".zsh_history",
];

/// Toolchain cache dirs mounted read-only before the workspace.
fn toolchain_caches() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = dirs_home() {
        for rel in [".cargo/registry", ".cargo/git", ".cache", ".rustup", "go/pkg", ".npm", ".local/share/pnpm"] {
            let p = home.join(rel);
            if p.is_dir() {
                out.push(p);
            }
        }
    }
    out
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
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
        let ws = cfg.workspace_dir.canonicalize()
            .unwrap_or_else(|_| cfg.workspace_dir.clone());

        let mut argv: Vec<String> = Vec::with_capacity(64);

        // ---- ro system binds ----
        for (src, dst) in [("/usr", "/usr"), ("/lib", "/lib"), ("/bin", "/bin"), ("/lib64", "/lib64")] {
            if Path::new(src).exists() {
                argv.push("--ro-bind".into());
                argv.push(src.into());
                argv.push(dst.into());
            }
        }
        // resolv.conf only when network is on.
        if cfg.allow_network && Path::new("/etc/resolv.conf").exists() {
            argv.push("--ro-bind".into());
            argv.push("/etc/resolv.conf".into());
            argv.push("/etc/resolv.conf".into());
        }
        argv.push("--proc".into());
        argv.push("/proc".into());
        argv.push("--dev".into());
        argv.push("/dev".into());

        // ---- sensitive dirs: shadowed by empty ro binds so they can't be read ----
        if let Some(home) = dirs_home() {
            for rel in SENSITIVE_DIRS {
                let p = home.join(rel);
                if p.exists() {
                    argv.push("--ro-bind".into());
                    argv.push("/dev/null".into());
                    argv.push(p.to_string_lossy().into_owned());
                }
            }
        }

        // ---- toolchain caches: ro, BEFORE the workspace bind (mount order) ----
        for cache in toolchain_caches() {
            argv.push("--ro-bind".into());
            argv.push(cache.to_string_lossy().into_owned());
            argv.push(cache.to_string_lossy().into_owned());
        }

        // ---- per-run tmp ----
        argv.push("--bind".into());
        argv.push(run_dir.to_string_lossy().into_owned());
        argv.push(run_dir.to_string_lossy().into_owned());

        // ---- workspace bind LAST (rw; CoW snapshot mounts at same path) ----
        argv.push("--bind".into());
        argv.push(ws.to_string_lossy().into_owned());
        argv.push(ws.to_string_lossy().into_owned());

        // ---- network ----
        if !cfg.allow_network {
            argv.push("--unshare-net".into());
        }

        // ---- PID namespace: fork-bomb containment (P2) ----
        // A private PID ns means a runaway fork tree dies with the sandbox's
        // init (bwrap's child) — it can't spread to host PIDs, and `--die-
        // with-parent` + timeout already reaps the whole namespace.
        argv.push("--unshare-pid".into());

        // ---- env: clearenv + sanitized set ----
        argv.push("--clearenv".into());
        let mut extra = cfg.env_vars.clone();
        extra.push(("CARGO_TARGET_DIR".into(), ws.join("target").to_string_lossy().into_owned()));
        extra.push(("TMPDIR".into(), run_dir.to_string_lossy().into_owned()));
        for (k, v) in sanitize_env(&extra) {
            argv.push("--setenv".into());
            argv.push(k);
            argv.push(v);
        }

        argv.push("--chdir".into());
        argv.push(ws.to_string_lossy().into_owned());
        argv.push("--die-with-parent".into());
        argv.push("--".into());
        // Absolute path — `--clearenv` may leave PATH unset inside the
        // namespace, so a bare `sh` can fail to execvp even though /bin/sh
        // exists (it does: we ro-bind /bin).
        argv.push("/bin/sh".into());
        argv.push("-c".into());
        // Prepend resource limits to the command (P2 — plan.rs's
        // ProcessPolicy was a dead field before). `ulimit -v` caps address
        // space (KB), `-u` caps processes for the UID in this ns. These run
        // inside the sandbox before the user command.
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

        // `wait_with_output` collects piped stdout/stderr — `child.wait()`
        // returns before the pipes drain, so reads after it see empty pipes.
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
            peak_memory_mb: None, // bwrap doesn't expose rusage; Stage 11 wires it
            elapsed_ms: elapsed,
        })
    }
}
