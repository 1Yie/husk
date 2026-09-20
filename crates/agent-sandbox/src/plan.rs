//! SandboxPlan — the structured OS-restriction set a `Policy` produces from
//! an `AuditResult`. It sits between policy ("is this allowed?") and the
//! platform backend ("how does the OS enforce it"): a plan is a portable
//! description of filesystem/network/process/device/syscall limits that a
//! Linux bwrap, macOS seatbelt, or landlock backend each translate to their
//! own mechanism. Policy never calls a Linux API directly.

use crate::capability::{Capability, RiskLevel};
use std::path::PathBuf;

/// Filesystem access the sandbox grants — the only writable roots.
#[derive(Debug, Clone)]
pub struct FilesystemPolicy {
    /// Read-write mounts (the workspace + a per-run tmp dir).
    pub rw: Vec<PathBuf>,
    /// Read-only mounts (system libs, toolchains).
    pub ro: Vec<PathBuf>,
    /// Explicitly denied paths (ssh keys, secrets, system dirs).
    pub deny: Vec<PathBuf>,
}

/// Network policy — on/off plus an optional allowlist of hosts.
#[derive(Debug, Clone)]
pub enum NetworkPolicy {
    /// No network (the default — `--unshare-net` / `deny network*`).
    Deny,
    /// Full network (a `Network`/`PackageInstall` capability flipped it on).
    Allow,
    /// Only these hosts reachable (DNS+connect filtered). Phase-2 — backends
    /// that can't filter fall back to `Allow` with a loud note.
    AllowHosts(Vec<String>),
}

/// Process/resource limits.
#[derive(Debug, Clone)]
pub struct ProcessPolicy {
    pub max_memory_mb: u64,
    pub timeout_secs: u64,
    /// Kill the whole process tree on timeout (cgroup/pgid scope).
    pub tree_kill: bool,
    /// Max spawned processes (fork-bomb guard).
    pub max_processes: u32,
}

/// Device node access — `/dev` is denied except the standard null/zero/urandom.
#[derive(Debug, Clone)]
pub enum DevicePolicy {
    /// Standard devices only (`/dev/null`, `/dev/zero`, `/dev/urandom`, tty).
    Standard,
    /// Extra device path allowed (rare — a DeviceAccess capability asked).
    AllowDevice(PathBuf),
}

/// Syscall filtering level (seccomp).
#[derive(Debug, Clone)]
pub enum SyscallPolicy {
    /// Baseline POSIX set — file I/O, process, memory, signals.
    Baseline,
    /// Baseline + network syscalls (when NetworkPolicy allows).
    Networked,
}

/// Environment surface the sandbox exposes — derived from the command's
/// `Runtime` capabilities (the agent asks for "a node runtime", never
/// for raw env names; the policy derives which bins/vars that means).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentPolicy {
    /// Baseline env — sanitized allowlist vars + system PATH only. No dev
    /// bins, no toolchain vars. For commands with no runtime needs.
    Minimal,
    /// Full dev toolchain — every discovered dev bin on PATH + toolchain
    /// vars (`VOLTA_HOME`, `CARGO_HOME`, …). The default while the sandbox
    /// is dev-friendly: a `Runtime` capability confirms the need, but
    /// plain commands keep it too (a `make` can invoke `cargo` internally
    /// — detection can't see inside build scripts).
    DevToolchain,
    /// Only the named toolchains — strict mode for future callers that
    /// trust their capability detection end-to-end. `Vec` of lowercase
    /// toolchain names (`"node"`, `"bun"`, `"rust"`).
    Select(Vec<String>),
}

/// The full plan a policy hands to a backend.
#[derive(Debug, Clone)]
pub struct SandboxPlan {
    pub filesystem: FilesystemPolicy,
    pub network: NetworkPolicy,
    pub processes: ProcessPolicy,
    pub devices: DevicePolicy,
    pub syscalls: SyscallPolicy,
    /// Environment surface — vars + PATH the spawn receives.
    pub environment: EnvironmentPolicy,
    /// CoW snapshot — critical-risk commands force `Required`.
    pub snapshot: crate::traits::SnapshotMode,
}

impl SandboxPlan {
    /// Build a plan from an `AuditResult`'s capabilities + risk. Workspace
    /// is always rw; the system dirs are always denied; capabilities toggle
    /// network/device/process on demand. This is the bridge between "what
    /// the command needs" and "what the OS will permit".
    pub fn from_audit(
        audit: &crate::audit::AuditResult,
        workspace: PathBuf,
        tmp_dir: PathBuf,
    ) -> Self {
        let needs_network = audit.capabilities.iter().any(|c| {
            matches!(c, Capability::Network { .. } | Capability::PackageInstall { .. })
        });
        let network_host = audit.capabilities.iter().find_map(|c| match c {
            Capability::Network { target: Some(t) } => Some(t.clone()),
            _ => None,
        });

        // Sensitive paths the sandbox always denies — capability-driven
        // policy can't widen these (OutOfScope/PrivilegeEscalation/Deny
        // capabilities keep them denied even when approved).
        let mut deny = vec![
            PathBuf::from("/etc/ssh"),
            PathBuf::from("/root"),
        ];
        for sub in [".ssh", ".gnupg", ".aws", ".config/agent-rs", ".config/husk"] {
            if let Some(p) = shellexpand_home(sub) {
                deny.push(p);
            }
        }

        Self {
            filesystem: FilesystemPolicy {
                rw: vec![workspace, tmp_dir],
                ro: vec![
                    PathBuf::from("/usr"),
                    PathBuf::from("/bin"),
                    PathBuf::from("/lib"),
                    PathBuf::from("/lib64"),
                ],
                deny,
            },
            network: if needs_network {
                match network_host {
                    Some(h) => NetworkPolicy::AllowHosts(vec![h]),
                    None => NetworkPolicy::Allow,
                }
            } else {
                NetworkPolicy::Deny
            },
            processes: ProcessPolicy {
                max_memory_mb: 2048,
                timeout_secs: 60,
                tree_kill: true,
                max_processes: 256,
            },
            devices: match audit.capabilities.iter().find_map(|c| match c {
                Capability::DeviceAccess { path } => Some(path.clone()),
                _ => None,
            }) {
                Some(p) => DevicePolicy::AllowDevice(p),
                None => DevicePolicy::Standard,
            },
            syscalls: if needs_network { SyscallPolicy::Networked } else { SyscallPolicy::Baseline },
            // Dev toolchain by default — a `Runtime` cap is confirmatory
            // signal, not a gate: detection can't see inside build scripts
            // (`make` can run `cargo`), so default-open is the correct
            // mode for a dev agent. `Minimal`/`Select` exist for future
            // strict callers that trust detection end-to-end.
            environment: EnvironmentPolicy::DevToolchain,
            snapshot: audit
                .force_snapshot
                .unwrap_or(if audit.risk >= RiskLevel::High {
                    crate::traits::SnapshotMode::Cow
                } else {
                    crate::traits::SnapshotMode::Off
                }),
        }
    }
}

fn shellexpand_home(sub: &str) -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(sub))
}
