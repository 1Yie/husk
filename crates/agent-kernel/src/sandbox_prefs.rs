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
    /// Master switch — `None`/`Some(true)` = sandboxed (default);
    /// `Some(false)` = every process tool runs through the loud
    /// `none` backend instead (no fs/net containment, no rlimits —
    /// the same contract as machines without bwrap).
    pub enabled: Option<bool>,
    /// `None` → the command's audit plan decides (default);
    /// `Some(true)` → force-allow network; `Some(false)` → force-deny.
    pub network: Option<bool>,
    /// Per-command resident memory cap (MB) — overrides the plan's 2048.
    pub max_memory_mb: Option<u64>,
    /// Fork-bomb guard (max spawned processes) — overrides the plan's 256.
    pub max_processes: Option<u32>,
    /// Standing egress allowlist — merged into every run's
    /// `SandboxConfig.allowed_domains` on top of what the audit derived.
    pub allowed_domains: Vec<String>,
    /// Standing egress denylist — wins over every allowlist entry.
    pub denied_domains: Vec<String>,
    /// Host-chosen directories mounted read-only into every sandboxed run
    /// (e.g. the user's skill roots). Kernel never picks these itself.
    pub extra_ro_mounts: Vec<std::path::PathBuf>,
}

impl SandboxLimits {
    /// Default is ON — a `None` slot means "the user never chose", not off.
    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

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
    enabled: None,
    network: None,
    max_memory_mb: None,
    max_processes: None,
    allowed_domains: Vec::new(),
    denied_domains: Vec::new(),
    extra_ro_mounts: Vec::new(),
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
    apply_limits(cfg, &current());
}

/// `apply_to` over an explicit snapshot — pure, so the precedence rules are
/// testable without touching the process-global overrides.
pub fn apply_limits(cfg: &mut SandboxConfig, l: &SandboxLimits) {
    if let Some(net) = l.network {
        cfg.allow_network = net;
    }
    if let Some(m) = l.max_memory_mb {
        cfg.max_memory_mb = m;
    }
    if let Some(p) = l.max_processes {
        cfg.max_processes = p;
    }
    if l.network == Some(false) {
        // Forced deny is authoritative: a non-empty allowlist makes the
        // backend route through the egress proxy instead of cutting the
        // network, so the plan's hosts must go too.
        cfg.allowed_domains.clear();
    } else {
        for d in &l.allowed_domains {
            if !cfg.allowed_domains.contains(d) {
                cfg.allowed_domains.push(d.clone());
            }
        }
    }
    for d in &l.denied_domains {
        if !cfg.denied_domains.contains(d) {
            cfg.denied_domains.push(d.clone());
        }
    }
    for m in &l.extra_ro_mounts {
        if !cfg.extra_ro_mounts.contains(m) {
            cfg.extra_ro_mounts.push(m.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An audit plan that granted package-manager hosts (`npm install`).
    fn cfg_with_plan_hosts() -> SandboxConfig {
        SandboxConfig {
            allow_network: false,
            allowed_domains: vec!["registry.npmjs.org".into()],
            ..Default::default()
        }
    }

    /// S2: "始终禁止" must mean no egress at all. Leaving the plan's host
    /// list in place makes the backend route through the proxy instead of
    /// cutting the network, so the forced deny was not a deny.
    #[test]
    fn forced_deny_drops_plan_and_standing_allowlists() {
        let mut cfg = cfg_with_plan_hosts();
        let limits = SandboxLimits {
            network: Some(false),
            allowed_domains: vec!["github.com".into()],
            ..Default::default()
        };
        apply_limits(&mut cfg, &limits);
        assert!(!cfg.allow_network);
        assert!(
            cfg.allowed_domains.is_empty(),
            "deny must leave nothing for the backend to proxy: {:?}",
            cfg.allowed_domains
        );
    }

    /// Control: auto keeps the plan's hosts and merges the standing list.
    #[test]
    fn auto_merges_allowlists_and_keeps_denylist() {
        let mut cfg = cfg_with_plan_hosts();
        let limits = SandboxLimits {
            allowed_domains: vec!["github.com".into()],
            denied_domains: vec!["evil.example".into()],
            ..Default::default()
        };
        apply_limits(&mut cfg, &limits);
        assert_eq!(cfg.allowed_domains, ["registry.npmjs.org", "github.com"]);
        assert_eq!(cfg.denied_domains, ["evil.example"]);
    }
}

/// The host's sandbox, honouring the live master switch.
///
/// The switch can flip mid-session (the settings page), so the choice is made
/// per call, not once at spawn: `enabled` reads the current overrides and
/// forwards to the real backend, or to `none` when isolation is off. `id` and
/// `tier` follow the same choice, so the UI never names a backend that is not
/// the one actually running the command.
pub struct SwitchableSandbox {
    real: std::sync::Arc<dyn agent_sandbox::SandboxBackend>,
    limits: std::sync::Arc<dyn Fn() -> SandboxLimits + Send + Sync>,
}

impl SwitchableSandbox {
    /// Reads the process-wide overrides on every call (product path).
    pub fn new(real: std::sync::Arc<dyn agent_sandbox::SandboxBackend>) -> Self {
        Self::with_limits(real, current)
    }

    /// Reads the switch from `limits` on every call — lets a host or test
    /// supply its own source instead of the process-global overrides.
    pub fn with_limits(
        real: std::sync::Arc<dyn agent_sandbox::SandboxBackend>,
        limits: impl Fn() -> SandboxLimits + Send + Sync + 'static,
    ) -> Self {
        Self {
            real,
            limits: std::sync::Arc::new(limits),
        }
    }

    fn active(&self) -> &dyn agent_sandbox::SandboxBackend {
        static NONE: agent_sandbox::NoneBackend = agent_sandbox::NoneBackend;
        if (self.limits)().enabled() {
            &*self.real
        } else {
            &NONE
        }
    }
}

#[async_trait::async_trait]
impl agent_sandbox::SandboxBackend for SwitchableSandbox {
    fn id(&self) -> &'static str {
        self.active().id()
    }

    fn tier(&self) -> agent_sandbox::SandboxTier {
        self.active().tier()
    }

    async fn run_command(
        &self,
        cmd: &str,
        args: &[&str],
        cfg: &SandboxConfig,
    ) -> anyhow::Result<agent_sandbox::CommandOutput> {
        self.active().run_command(cmd, args, cfg).await
    }

    fn wrap_spawn(
        &self,
        cfg: &SandboxConfig,
        cmd: &str,
        args: &[&str],
    ) -> Option<tokio::process::Command> {
        self.active().wrap_spawn(cfg, cmd, args)
    }
}
