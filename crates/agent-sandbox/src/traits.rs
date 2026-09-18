//! The `SandboxBackend` trait + shared config/output types.
//!
//! Contract (sandbox-model.md): every agent command runs inside an enforced
//! boundary; a missing backend degrades **loudly** (`id() == "none"`), never
//! silently. Three tiers: L1 WASM (plugin-system), L2 native process
//! (this crate), L3 microVM (future impl of the same trait).

use std::path::PathBuf;

/// Which isolation tier a backend implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxTier {
    /// In-process WASM (plugin tools).
    L1,
    /// Native process sandbox (bwrap / sandbox-exec / job-object).
    L2,
    /// MicroVM (Firecracker/QEMU) — exceptional workloads only.
    L3,
}

/// Filesystem snapshot behavior for a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotMode {
    /// Write directly to the real workspace.
    Off,
    /// Clone-on-write: agent mutates a reflink copy, merge back on success.
    Cow,
    /// Every write-bearing command must run against a CoW snapshot.
    Required,
}

/// Per-run configuration — the caller builds this per tool call.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Canonical workspace root — the only rw mount inside the sandbox.
    pub workspace_dir: PathBuf,
    /// Whether the sandboxed process may reach the network.
    pub allow_network: bool,
    /// Max resident memory (MB) — 2048 default.
    pub max_memory_mb: u64,
    /// Max spawned processes (fork-bomb guard) — backends that can enforce
    /// it do (bwrap: `--unshare-pid` + `ulimit -u`); 0 = no explicit cap.
    pub max_processes: u32,
    /// Wall-clock timeout (s) — caller may raise to ≤ 600.
    pub timeout_secs: u64,
    /// Extra env vars to inject *after* sanitization (e.g. PATH overrides).
    pub env_vars: Vec<(String, String)>,
    /// CoW snapshot behavior.
    pub snapshot: SnapshotMode,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            workspace_dir: PathBuf::from("."),
            allow_network: false,
            max_memory_mb: 2048,
            max_processes: 256,
            timeout_secs: 60,
            env_vars: Vec::new(),
            snapshot: SnapshotMode::Off,
        }
    }
}

/// What a sandboxed run produced.
#[derive(Debug)]
pub struct CommandOutput {
    /// Process exit status.
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
    /// True when the wall timeout fired (tree killed).
    pub is_timeout: bool,
    /// Peak RSS if the platform reports it.
    pub peak_memory_mb: Option<u64>,
    /// Wall-clock elapsed.
    pub elapsed_ms: u64,
}

/// One sandbox backend. `run_command` is the only entry — all four control
/// dimensions (fs isolation, network, resource limits, env sanitization)
/// live behind it.
#[async_trait::async_trait]
pub trait SandboxBackend: Send + Sync {
    /// Stable id for telemetry/UI (`"bwrap"`, `"landlock"`, `"none"`…).
    fn id(&self) -> &'static str;
    /// Isolation tier.
    fn tier(&self) -> SandboxTier;
    /// Run `cmd` with `args` inside the sandbox. The backend applies env
    /// sanitization + resource limits + timeout-tree-kill itself — callers
    /// must not pre-sanitize.
    async fn run_command(
        &self,
        cmd: &str,
        args: &[&str],
        cfg: &SandboxConfig,
    ) -> anyhow::Result<CommandOutput>;
}
