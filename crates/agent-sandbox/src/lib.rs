//! agent-sandbox — L2 process sandbox.
//!
//! `detect_backend()` picks the best implemented backend — bwrap on Linux — and
//! otherwise returns the loud `none` backend. Every spawn gets env sanitization,
//! resource limits and tree-kill on timeout regardless of backend.


pub mod audit;
pub mod env_sanitize;
pub mod linux_bwrap;
pub mod none;
pub mod traits;

pub use audit::{audit_command, audit_command_scoped, AuditLevel, AuditResult, AuditVerdict};
pub use env_sanitize::{is_denied, sanitize_env};
pub use linux_bwrap::LinuxBwrap;
pub use none::NoneBackend;
pub use traits::{CommandOutput, SandboxBackend, SandboxConfig, SandboxTier, SnapshotMode};

/// Pick the best backend for this platform at startup.
///
/// Linux → bwrap when it is on PATH, otherwise `none`.
///
/// Returns the backend + a `loud` flag — `none` must surface the
/// `UNSANDBOXED` chip and force-confirm every `bash` call.
pub fn detect_backend() -> (Box<dyn SandboxBackend>, bool) {
    #[cfg(target_os = "linux")]
    {
        if bwrap_on_path() {
            return (Box::new(LinuxBwrap), false);
        }
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

/// Shell AST — structural parse of a command line (parser layer of the
/// sandbox pipeline; safety decisions live in capability/audit/policy).
pub mod shell_ast;

/// Capability IR + extractor — `shell_ast` → `Vec<Capability>` + `RiskLevel`.
pub mod capability;

/// SandboxPlan — the OS-portable restriction set policy hands to a backend.
pub mod plan;
