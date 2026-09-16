//! agent-sandbox — L2 process sandbox.
//!
//! Phase 1 (this stage): `traits.rs` + `linux_bwrap.rs` + `none.rs` +
//! `audit.rs` + `env_sanitize.rs`. `detect_backend()` picks the best
//! available: bwrap → landlock (Phase 2) → none (loud fallback).
//!
//! Every spawn gets env sanitization, resource limits, and tree-kill on
//! timeout regardless of backend — containment may degrade, leakage never.

pub mod audit;
pub mod env_sanitize;
pub mod linux_bwrap;
pub mod none;
pub mod traits;

pub use audit::{audit_command, AuditLevel, AuditVerdict};
pub use env_sanitize::{is_denied, sanitize_env};
pub use linux_bwrap::LinuxBwrap;
pub use none::NoneBackend;
pub use traits::{CommandOutput, SandboxBackend, SandboxConfig, SandboxTier, SnapshotMode};

/// Pick the best backend for this platform at startup.
///
/// Linux → bwrap (if on PATH) → landlock (Phase 2) → none.
/// macOS → sandbox-exec (Phase 2) → none. Windows → job-object (Phase 3).
///
/// Returns the backend + a `loud` flag — `none` must surface the
/// `UNSANDBOXED` chip and force-confirm every `bash` call.
pub fn detect_backend() -> (Box<dyn SandboxBackend>, bool) {
    #[cfg(target_os = "linux")]
    {
        if bwrap_on_path() {
            return (Box::new(LinuxBwrap), false);
        }
        // Phase 2 adds landlock here (zero-dep kernel ≥5.13 fallback).
        return (Box::new(NoneBackend), true);
    }
    #[cfg(not(target_os = "linux"))]
    {
        // macOS sandbox-exec / Windows job-object land in Phase 2/3.
        (Box::new(NoneBackend), true)
    }
}

fn bwrap_on_path() -> bool {
    std::process::Command::new("bwrap")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
