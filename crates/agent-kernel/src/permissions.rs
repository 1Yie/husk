//! `PermissionGate` — the policy engine between "model asked for a tool" and
//! "tool executes".
//!
//! Contract (kernel-architecture.md §Permission modes):
//!
//! | Mode                 | Behavior                                              |
//! |----------------------|-------------------------------------------------------|
//! | `default`            | ask for non-readonly tools; readonly auto-run         |
//! | `acceptEdits`        | file edits auto-approve; shell still asks             |
//! | `auto`               | auto-approve what passes safety checks; escalate rest |
//! | `dontAsk`            | only pre-approved tools + readonly shell              |
//! | `bypassPermissions`  | approve all except `deny` rules + destructive `ask`   |
//!
//! Precedence: `deny > ask > allow`. Rules come from CLI flags +
//! `~/.config/<app>/config.toml` + `<repo>/.agent/config.toml` (Stage 6
//! wires the repo-local file; global config lands with the CLI in Stage 7).
//!
//! The gate is *synchronous policy*, not a channel — the engine calls
//! `decide()` and, on `Ask`, pauses in `AwaitingToolConfirmation` until the
//! session's `ToolDecision` command resolves it.

use std::collections::HashSet;

/// The five modes — `from_str` accepts the labels the prompt/config use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    Default,
    AcceptEdits,
    Auto,
    DontAsk,
    Bypass,
}

impl PermissionMode {
    pub fn from_str(s: &str) -> Self {
        match s {
            "acceptEdits" | "accept-edits" => Self::AcceptEdits,
            "auto" => Self::Auto,
            "dontAsk" | "dont-ask" => Self::DontAsk,
            "bypassPermissions" | "bypass" => Self::Bypass,
            _ => Self::Default,
        }
    }
}

/// What the gate returns for one tool call.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Run it — no pause.
    Allow,
    /// Refuse outright (a `deny` rule, or `dontAsk` with no pre-approval).
    Deny { reason: String },
    /// Pause in `AwaitingToolConfirmation`; UI decides.
    Ask { diff_summary: String },
}

/// Rule sets — `deny` always wins, then `ask`, then `allow`.
#[derive(Debug, Default)]
pub struct PermissionRules {
    /// Tool names that never run (e.g. `"bash"`, `"fuzzy_patch"`).
    pub deny: HashSet<String>,
    /// Tool names that always pause for confirmation.
    pub ask: HashSet<String>,
    /// Tool names pre-approved (skip the pause in `default`/`dontAsk`).
    pub allow: HashSet<String>,
}

/// Readonly shell verbs for the `dontAsk` whitelist — `bash` is non-readonly
/// by spec, but these command prefixes count as readonly shell.
const READONLY_SHELL: &[&str] = &[
    "ls", "cat", "head", "tail", "grep", "rg", "find", "pwd", "wc",
    "git status", "git diff", "git log", "git show", "git branch",
];

/// Destructive shell verbs that `bypassPermissions` still escalates to `Ask`.
const DESTRUCTIVE_SHELL: &[&str] = &[
    "rm", "mv", "dd", "mkfs", "shutdown", "reboot", "kill", "pkill",
    "git reset --hard", "git clean -f", "git checkout --",
];

pub struct PermissionGate {
    mode: PermissionMode,
    rules: PermissionRules,
}

impl PermissionGate {
    pub fn new(mode: PermissionMode, rules: PermissionRules) -> Self {
        Self { mode, rules }
    }

    pub fn from_mode_str(mode: &str) -> Self {
        Self::new(PermissionMode::from_str(mode), PermissionRules::default())
    }

    pub fn mode(&self) -> PermissionMode {
        self.mode
    }

    /// Merge repo-local rules (`.agent/config.toml` → deny/ask/allow lists).
    pub fn merge_rules(&mut self, rules: PermissionRules) {
        self.rules.deny.extend(rules.deny);
        self.rules.ask.extend(rules.ask);
        self.rules.allow.extend(rules.allow);
    }

    /// Decide one tool call. `tool_name` + `is_readonly` come from the
    /// registry; `command` is the shell text for `bash`/`pty` calls (empty
    /// for file tools); `diff_summary` is pre-computed for `Ask` cards.
    ///
    /// Precedence: deny > ask > allow > mode default.
    pub fn decide(
        &self,
        tool_name: &str,
        is_readonly: bool,
        command: Option<&str>,
        diff_summary: impl Into<String>,
    ) -> Decision {
        // 1. Explicit deny — absolute.
        if self.rules.deny.contains(tool_name) {
            return Decision::Deny { reason: format!("`{tool_name}` denied by rule") };
        }
        // Destructive shell always asks in every mode except a matching allow.
        if let Some(cmd) = command {
            if is_destructive(cmd) && !self.rules.allow.contains(tool_name) {
                return Decision::Ask { diff_summary: diff_summary.into() };
            }
        }
        // 2. Explicit ask.
        if self.rules.ask.contains(tool_name) {
            return Decision::Ask { diff_summary: diff_summary.into() };
        }
        // 3. Explicit allow.
        if self.rules.allow.contains(tool_name) {
            return Decision::Allow;
        }
        // 4. Mode default.
        match self.mode {
            PermissionMode::Bypass => Decision::Allow,
            PermissionMode::Default => {
                if is_readonly { Decision::Allow } else { Decision::Ask { diff_summary: diff_summary.into() } }
            }
            PermissionMode::AcceptEdits => {
                if is_readonly || is_file_edit(tool_name) {
                    Decision::Allow
                } else {
                    Decision::Ask { diff_summary: diff_summary.into() }
                }
            }
            PermissionMode::Auto => {
                // auto-approve readonly + file edits; escalate shell/unknown.
                if is_readonly || is_file_edit(tool_name) {
                    Decision::Allow
                } else {
                    Decision::Ask { diff_summary: diff_summary.into() }
                }
            }
            PermissionMode::DontAsk => {
                if is_readonly || self.rules.allow.contains(tool_name) {
                    Decision::Allow
                } else if let Some(cmd) = command {
                    // shell: only readonly verbs pass.
                    if is_readonly_shell(cmd) {
                        Decision::Allow
                    } else {
                        Decision::Deny { reason: "dontAsk: non-readonly shell".into() }
                    }
                } else {
                    Decision::Deny { reason: "dontAsk: tool not pre-approved".into() }
                }
            }
        }
    }
}

/// File-edit tools — `acceptEdits`/`auto` approve these without asking.
fn is_file_edit(tool_name: &str) -> bool {
    matches!(tool_name, "fuzzy_patch" | "apply_patch" | "write_file")
}

/// Command starts with a readonly-shell verb?
fn is_readonly_shell(cmd: &str) -> bool {
    let c = cmd.trim();
    READONLY_SHELL.iter().any(|v| c.starts_with(v))
}

/// Command contains a destructive verb?
fn is_destructive(cmd: &str) -> bool {
    let c = cmd.trim();
    DESTRUCTIVE_SHELL.iter().any(|v| c.contains(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(mode: &str) -> PermissionGate {
        PermissionGate::from_mode_str(mode)
    }

    #[test]
    fn default_asks_for_writes_not_reads() {
        let g = gate("default");
        assert!(matches!(
            g.decide("smart_read", true, None, ""),
            Decision::Allow
        ));
        assert!(matches!(
            g.decide("fuzzy_patch", false, None, "engine.rs +5"),
            Decision::Ask { .. }
        ));
    }

    #[test]
    fn deny_beats_everything() {
        let mut g = gate("bypassPermissions");
        g.merge_rules(PermissionRules {
            deny: ["bash".into()].into_iter().collect(),
            ..Default::default()
        });
        assert!(matches!(
            g.decide("bash", false, Some("ls"), ""),
            Decision::Deny { .. }
        ));
    }

    #[test]
    fn destructive_shell_asks_even_in_bypass() {
        let g = gate("bypassPermissions");
        assert!(matches!(
            g.decide("bash", false, Some("rm -rf target"), ""),
            Decision::Ask { .. }
        ));
    }

    #[test]
    fn accept_edits_skips_patch_asks_shell() {
        let g = gate("acceptEdits");
        assert!(matches!(g.decide("fuzzy_patch", false, None, ""), Decision::Allow));
        assert!(matches!(
            g.decide("bash", false, Some("cargo test"), ""),
            Decision::Ask { .. }
        ));
    }

    #[test]
    fn dont_ask_allows_readonly_shell_denies_write() {
        let g = gate("dontAsk");
        assert!(matches!(
            g.decide("bash", false, Some("git status"), ""),
            Decision::Allow
        ));
        assert!(matches!(
            g.decide("bash", false, Some("cargo build"), ""),
            Decision::Deny { .. }
        ));
    }
}
