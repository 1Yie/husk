//! `mac_sandbox_exec.rs` — macOS Seatbelt backend via `sandbox-exec`.
//!
//! One-paragraph posture: the child runs under `(deny default)` plus
//! targeted allows — system libs read-only, workspace+tmp rw, the usual
//! secret dirs denied, network gated by `allow_network`. Resource limits
//! Seatbelt can't express (memory caps, process counts) stay unenforced;
//! the timeout tree-kill is the harness's, same as `none`.
//!
//! UNTESTED-on-CI note: this file compiles only on macOS. The profile is
//! conservative — when in doubt an operation is denied rather than allowed,
//! which fails loudly (the child exits) instead of silently widening.

#![cfg(target_os = "macos")]

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};

use crate::env_sanitize::sanitize_env;
use crate::traits::{CommandOutput, SandboxBackend, SandboxConfig, SandboxTier};

pub struct MacSandboxExec;

/// SBPL profile for this run — `workspace_dir`, scratch roots and the
/// cfg's extra mounts baked in as `(subpath)` literals.
fn sbpl(cfg: &SandboxConfig, ws: &Path) -> String {
    let mut p = String::from("(version 1)(deny default)");
    // Process machinery — exec/fork/signal within the sandboxed family.
    p.push_str("(allow process-exec process-fork (allow signal (target same-sandbox)))");
    // Mach + sysctl: runtimes (dyld, CF, node/python) look up services and
    // sysctls during startup — denying these breaks almost everything.
    p.push_str("(allow mach-lookup sysctl-read sysctl-write (regex \"^hw\\.\") iokit-open)");
    // System trees: executables + dynamic libs + config the linker reads.
    for d in [
        "/usr",
        "/bin",
        "/sbin",
        "/opt",
        "/Library",
        "/System/Library",
        "/private/etc",
        "/private/var/db",
        "/usr/share",
        "/var",
        "/Applications",
        "/System/Applications",
        "/Library/Apple",
    ] {
        p.push_str(&format!(
            "(allow file-read* file-map-executable (subpath \"{d}\"))"
        ));
    }
    // Scratch + pseudo-fs.
    for d in ["/tmp", "/private/tmp", "/private/var/folders", "/var/tmp"] {
        p.push_str(&format!(
            "(allow file-read* file-write* file-map-executable (subpath \"{d}\"))"
        ));
    }
    p.push_str(
        "(allow file-read* file-write* (literal \"/dev/null\") (literal \"/dev/zero\") \
         (literal \"/dev/urandom\") (literal \"/dev/random\") (literal \"/dev/tty\") \
         (regex \"^/dev/ptmx\") (regex \"^/dev/ttys\"))",
    );
    // Caller mounts.
    for d in &cfg.extra_ro_mounts {
        if let Some(d) = d.to_str() {
            p.push_str(&format!(
                "(allow file-read* file-map-executable (subpath \"{d}\"))"
            ));
        }
    }
    for d in &cfg.extra_rw_mounts {
        if let Some(d) = d.to_str() {
            p.push_str(&format!(
                "(allow file-read* file-write* file-map-executable (subpath \"{d}\"))"
            ));
        }
    }
    // Workspace — read-write, last fs rule before the deny overrides.
    if let Some(w) = ws.to_str() {
        p.push_str(&format!(
            "(allow file-read* file-write* file-map-executable (subpath \"{w}\"))"
        ));
    }
    // Secrets — deny AFTER the allows: seatbelt's last-match wins, so these
    // shadow whatever an allow would have granted under $HOME.
    if let Some(home) = std::env::var_os("HOME").and_then(|h| h.into_string().ok()) {
        for sub in [
            ".ssh",
            ".gnupg",
            ".aws",
            ".azure",
            ".kube",
            ".npmrc",
            ".netrc",
            "Library/Keychains",
            ".config/agent-rs",
            ".config/husk",
            ".cargo/credentials",
            ".cargo/credentials.toml",
        ] {
            p.push_str(&format!(
                "(deny file-read* file-write* file-map-executable (subpath \"{home}/{sub}\"))"
            ));
        }
    }
    // Network — all-or-nothing (seatbelt can't host-filter either).
    if cfg.allow_network {
        p.push_str("(allow network* system-socket)");
    } else {
        p.push_str("(deny network* )");
    }
    p
}

#[async_trait::async_trait]
impl SandboxBackend for MacSandboxExec {
    fn id(&self) -> &'static str {
        "sandbox-exec"
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
        let timeout = std::time::Duration::from_secs(cfg.timeout_secs.min(600));
        let ws = cfg
            .workspace_dir
            .canonicalize()
            .unwrap_or_else(|_| cfg.workspace_dir.clone());
        let mut envs = sanitize_env(&cfg.env_vars);
        for (k, v) in &cfg.trusted_env {
            envs.push((k.clone(), v.clone()));
        }
        let profile = sbpl(cfg, &ws);

        let mut command = tokio::process::Command::new("/usr/bin/sandbox-exec");
        command
            .arg("-p")
            .arg(&profile)
            .arg("/bin/sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(&ws)
            .env_clear()
            .envs(envs)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let child = command.spawn().context("spawn sandbox-exec")?;
        let child_id = child.id();
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
            Err(_) => {
                if let Some(id) = child_id {
                    let _ = std::process::Command::new("kill")
                        .args(["-9", &id.to_string()])
                        .status();
                }
                (String::new(), "[timeout — process killed]".into(), -1, true)
            }
        };
        Ok(CommandOutput {
            status: code,
            stdout: out,
            stderr: err,
            is_timeout,
            peak_memory_mb: None,
            elapsed_ms: elapsed,
        })
    }

    fn wrap_spawn(
        &self,
        cfg: &SandboxConfig,
        cmd: &str,
        args: &[&str],
    ) -> Option<tokio::process::Command> {
        let ws = cfg
            .workspace_dir
            .canonicalize()
            .unwrap_or_else(|_| cfg.workspace_dir.clone());
        let profile = sbpl(cfg, &ws);
        let mut c = tokio::process::Command::new("/usr/bin/sandbox-exec");
        c.arg("-p").arg(&profile).arg(cmd).args(args);
        let mut envs = sanitize_env(&cfg.env_vars);
        for (k, v) in &cfg.trusted_env {
            envs.push((k.clone(), v.clone()));
        }
        c.env_clear().envs(envs);
        Some(c)
    }
}

/// `sandbox-exec` shipped with macOS — absent ⇒ fall through to `none`.
pub fn sandbox_exec_on_path() -> bool {
    std::process::Command::new("/usr/bin/sandbox-exec")
        .arg("-h")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
