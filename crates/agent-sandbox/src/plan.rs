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

/// The full plan a policy hands to a backend.
#[derive(Debug, Clone)]
pub struct SandboxPlan {
    pub filesystem: FilesystemPolicy,
    pub network: NetworkPolicy,
    pub processes: ProcessPolicy,
    pub devices: DevicePolicy,
    pub syscalls: SyscallPolicy,
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
        for sub in [".ssh", ".gnupg", ".aws", ".config/agent-rs"] {
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
