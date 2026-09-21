//! `sandbox_prefs` — process-wide sandbox overrides from the settings UI.
//!
//! Process tools (`bash`, `test_runner`) fold these into the per-run
//! `SandboxConfig` AFTER the audit plan fills its defaults — `None` fields
//! leave the plan's values untouched.

use std::sync::RwLock;

use agent_sandbox::SandboxConfig;

/// User overrides for sandboxed command execution.
#[derive(Debug, Clone, Default)]
pub struct SandboxLimits {
    /// `None` → the command's audit plan decides (default);
    /// `Some(true)` → force-allow network; `Some(false)` → force-deny.
    pub network: Option<bool>,
    /// Per-command resident memory cap (MB) — overrides the plan's 2048.
    pub max_memory_mb: Option<u64>,
    /// Fork-bomb guard (max spawned processes) — overrides the plan's 256.
    pub max_processes: Option<u32>,
}

impl SandboxLimits {
    /// `"auto"` / `"allow"` / `"deny"` label for persistence + the settings UI.
    pub fn network_label(&self) -> &'static str {
        match self.network {
            Some(true) => "allow",
            Some(false) => "deny",
            None => "auto",
        }
    }
}

/// Parse a persisted/settings label back into the tri-state.
pub fn network_from_label(s: &str) -> Option<bool> {
    match s {
        "allow" => Some(true),
        "deny" => Some(false),
        _ => None,
    }
}

static LIMITS: RwLock<SandboxLimits> = RwLock::new(SandboxLimits {
    network: None,
    max_memory_mb: None,
    max_processes: None,
});

/// Current overrides — a poisoned read yields defaults rather than
/// panicking inside a tool call.
pub fn current() -> SandboxLimits {
    LIMITS.read().map(|l| l.clone()).unwrap_or_default()
}

/// Replace the overrides — seeded from persisted prefs at kernel boot and
/// updated by `set_default_prefs`.
pub fn set(limits: SandboxLimits) {
    if let Ok(mut w) = LIMITS.write() {
        *w = limits;
    }
}

/// Fold the overrides into a per-run `SandboxConfig` after the audit plan
/// filled its defaults.
pub fn apply_to(cfg: &mut SandboxConfig) {
    let l = current();
    if let Some(net) = l.network {
        cfg.allow_network = net;
    }
    if let Some(m) = l.max_memory_mb {
        cfg.max_memory_mb = m;
    }
    if let Some(p) = l.max_processes {
        cfg.max_processes = p;
    }
}
