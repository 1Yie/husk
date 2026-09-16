//! Pre-execution audit — pattern-match `bash` commands *before* spawn.
//!
//! Contract (sandbox-model.md §Pre-execution audit): feeds
//! `PendingApprovalData.risk` and can force `snapshot: Required`. Audit is
//! string analysis — it complements the OS sandbox, never replaces it. A
//! critical command the user approves still runs sandboxed (with CoW).

use crate::traits::SnapshotMode;

/// Risk tier surfaced to the approval card + sandbox config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuditLevel {
    /// Normal — permission mode decides.
    Normal,
    /// Package-manager mutation; also forces `allow_network` prompt.
    NetworkMutating,
    /// Confirmation required in every mode incl. `acceptEdits`.
    Elevated,
    /// Red card — only runs after explicit approve + forced `Required` CoW.
    Critical,
    /// Command targets paths outside the sandbox — deny unless widened.
    ScopeViolation,
}

#[derive(Debug, Clone)]
pub struct AuditVerdict {
    pub level: AuditLevel,
    /// The pattern that matched (shown on the approval card).
    pub reason: String,
    /// Audit-mandated snapshot upgrade (Critical forces Required).
    pub force_snapshot: Option<SnapshotMode>,
    /// Whether the network prompt must accompany confirmation.
    pub needs_network_prompt: bool,
}

/// Patterns → level, first match wins. Ordered Critical → Normal.
const RULES: &[(&str, AuditLevel)] = &[
    // Critical — destructive to the host.
    ("rm -rf /", AuditLevel::Critical),
    ("rm -fr /", AuditLevel::Critical),
    ("rm -rf ~", AuditLevel::Critical),
    ("rm -rf $HOME", AuditLevel::Critical),
    ("mkfs", AuditLevel::Critical),
    ("dd of=/dev", AuditLevel::Critical),
    ("> /dev/sd", AuditLevel::Critical),
    (":(){", AuditLevel::Critical), // fork bomb
    ("chmod -R 777 /", AuditLevel::Critical),
    // Elevated — dangerous but bounded.
    ("git push --force", AuditLevel::Elevated),
    ("git push -f", AuditLevel::Elevated),
    ("git reset --hard", AuditLevel::Elevated),
    ("sudo ", AuditLevel::Elevated),
    ("chmod -R 777", AuditLevel::Elevated),
    ("curl ", AuditLevel::Elevated),   // curl|sh patterns covered below
    ("wget ", AuditLevel::Elevated),
    ("| sh", AuditLevel::Elevated),
    ("| bash", AuditLevel::Elevated),
    // Network-mutating — package installs.
    ("npm install", AuditLevel::NetworkMutating),
    ("cargo add", AuditLevel::NetworkMutating),
    ("brew install", AuditLevel::NetworkMutating),
    ("pip install", AuditLevel::NetworkMutating),
    ("apt install", AuditLevel::NetworkMutating),
    ("apt-get install", AuditLevel::NetworkMutating),
    // Scope violations — write paths outside the sandbox.
    ("> ~/.", AuditLevel::ScopeViolation),
    ("> $HOME", AuditLevel::ScopeViolation),
    ("tee ~/.", AuditLevel::ScopeViolation),
];

/// Audit one command line. Returns the verdict; the caller maps `level` to
/// `PendingApprovalData.risk` and merges `force_snapshot` into the config.
pub fn audit_command(cmd: &str) -> AuditVerdict {
    for (pat, level) in RULES {
        if cmd.contains(pat) {
            return verdict(*level, pat);
        }
    }
    AuditVerdict {
        level: AuditLevel::Normal,
        reason: String::new(),
        force_snapshot: None,
        needs_network_prompt: false,
    }
}

fn verdict(level: AuditLevel, pat: &str) -> AuditVerdict {
    AuditVerdict {
        level,
        reason: format!("matched `{pat}`"),
        force_snapshot: match level {
            AuditLevel::Critical => Some(SnapshotMode::Required),
            _ => None,
        },
        needs_network_prompt: level == AuditLevel::NetworkMutating,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rm_rf_root_is_critical_with_required_snapshot() {
        let v = audit_command("rm -rf /");
        assert_eq!(v.level, AuditLevel::Critical);
        assert_eq!(v.force_snapshot, Some(SnapshotMode::Required));
    }

    #[test]
    fn force_push_is_elevated() {
        assert_eq!(audit_command("git push --force origin main").level, AuditLevel::Elevated);
        assert_eq!(audit_command("sudo apt update").level, AuditLevel::Elevated);
    }

    #[test]
    fn npm_install_needs_network_prompt() {
        let v = audit_command("npm install lodash");
        assert_eq!(v.level, AuditLevel::NetworkMutating);
        assert!(v.needs_network_prompt);
    }

    #[test]
    fn clean_command_is_normal() {
        assert_eq!(audit_command("cargo test").level, AuditLevel::Normal);
        assert_eq!(audit_command("ls -la").level, AuditLevel::Normal);
    }
}
