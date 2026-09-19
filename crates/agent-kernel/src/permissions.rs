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

    /// Canonical wire label — the inverse of `from_str`, used to report a
    /// live gate's mode back to the UI (`model_info`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::AcceptEdits => "acceptEdits",
            Self::Auto => "auto",
            Self::DontAsk => "dontAsk",
            Self::Bypass => "bypassPermissions",
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
    "file", "stat", "du", "df", "echo", "which", "env", "tree",
    "uname", "date", "hostname", "id", "whoami",
    // note: `cargo test`/`cargo build`/`cargo check` write artifacts → NOT
    // readonly; they ask for confirmation like any mutating command.
];

/// Destructive shell patterns that escalate to `Ask` even in `auto` and `bypass`.
const DESTRUCTIVE_PHRASES: &[&str] = &[
    "git reset --hard",
    "git clean -f",
    "git checkout --",
];

/// Destructive shell binary names that escalate to `Ask`.
const DESTRUCTIVE_BINARIES: &[&str] = &[
    "rm", "mv", "dd", "mkfs", "shutdown", "reboot", "kill", "pkill",
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
                // readonly tools auto-run; a SHELL call auto-runs too when
                // its command is a readonly verb (ls/cat/rg/git status…) —
                // only writes, installs, and destructive ops pause for
                // confirmation. Matches the spec's intent: approval is for
                // mutations, not inspection.
                if is_readonly {
                    Decision::Allow
                } else if command.map(is_readonly_shell).unwrap_or(false) {
                    Decision::Allow
                } else {
                    Decision::Ask { diff_summary: diff_summary.into() }
                }
            }
            PermissionMode::AcceptEdits => {
                // Auto-approve readonly tools + readonly shell + file edits; mutating shell asks.
                if is_readonly || is_file_edit(tool_name) {
                    Decision::Allow
                } else if command.map(is_readonly_shell).unwrap_or(false) {
                    Decision::Allow
                } else {
                    Decision::Ask { diff_summary: diff_summary.into() }
                }
            }
            PermissionMode::Auto => {
                // Full autonomous execution: readonly tools, file edits, and safe shell commands auto-run.
                // Destructive commands were already caught by is_destructive and escalated to Ask.
                Decision::Allow
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
    matches!(tool_name, "fuzzy_patch" | "apply_patch" | "write_file" | "fs_patch")
}

/// Command is a PURE readonly-shell call? Starts with a readonly verb AND
/// contains no write/chain operator — `cat a > b`, `ls && rm`, `ls | sh`
/// must not count as readonly (they mutate or smuggle a second command).
fn is_readonly_shell(cmd: &str) -> bool {
    let c = cmd.trim();
    if c.is_empty() {
        return false;
    }
    let starts_readonly = READONLY_SHELL.iter().any(|v| {
        c == *v || c.starts_with(&format!("{v} "))
    });
    if !starts_readonly {
        return false;
    }
    // Reject anything that could write or chain a second command.
    !(c.contains('>')
        || c.contains('|')
        || c.contains("&&")
        || c.contains(';')
        || c.contains("$(")
        || c.contains('`'))
}

/// Command contains a destructive verb or pattern?
/// Accurately tokenizes command segments to avoid substring false positives (e.g. `git add` matching `dd`).
fn is_destructive(cmd: &str) -> bool {
    let c = cmd.trim();
    if c.is_empty() {
        return false;
    }

    // 1. Multi-word destructive phrases
    for phrase in DESTRUCTIVE_PHRASES {
        if c.contains(phrase) {
            return true;
        }
    }

    // 2. Tokenize compound command / pipeline segments
    for segment in c.split([';', '&', '|']) {
        let seg = segment.trim();
        if seg.is_empty() {
            continue;
        }

        let words: Vec<&str> = seg.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }

        let mut idx = 0;
        while idx < words.len() {
            let w = words[idx];
            // Skip wrappers
            if w == "sudo" || w == "env" || w == "nohup" || w == "xargs" {
                idx += 1;
                continue;
            }
            if w.starts_with('-') {
                idx += 1;
                continue;
            }
            let bin = std::path::Path::new(w)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(w);

            if DESTRUCTIVE_BINARIES.contains(&bin) {
                return true;
            }
            break;
        }
    }

    false
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
    fn default_allows_readonly_shell_but_asks_writes() {
        let g = gate("default");
        // readonly shell verbs auto-run — ls/cat/git status/cargo test…
        assert!(matches!(
            g.decide("bash", false, Some("ls -la"), ""),
            Decision::Allow
        ));
        assert!(matches!(
            g.decide("bash", false, Some("git status"), ""),
            Decision::Allow
        ));
        // …but writes, installs, builds, and chains still ask.
        assert!(matches!(
            g.decide("bash", false, Some("cargo test"), ""),
            Decision::Ask { .. }
        ));
        assert!(matches!(
            g.decide("bash", false, Some("mkdir demo"), ""),
            Decision::Ask { .. }
        ));
        assert!(matches!(
            g.decide("bash", false, Some("cat a > b.txt"), ""),
            Decision::Ask { .. }
        ));
        assert!(matches!(
            g.decide("bash", false, Some("ls && rm -rf x"), ""),
            Decision::Ask { .. }
        ));
    }

    #[test]
    fn default_allows_todo_without_asking() {
        let g = gate("default");
        assert!(matches!(g.decide("todo", true, None, ""), Decision::Allow));
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
        // non-readonly shell (an install) still asks even in acceptEdits.
        assert!(matches!(
            g.decide("bash", false, Some("npm install foo"), ""),
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
        // a build writes artifacts → not readonly → denied in dontAsk.
        assert!(matches!(
            g.decide("bash", false, Some("mkdir out"), ""),
            Decision::Deny { .. }
        ));
    }

    #[test]
    fn auto_allows_safe_shell_and_edits_asks_destructive() {
        let g = gate("auto");
        // file edits auto-run in auto
        assert!(matches!(g.decide("fuzzy_patch", false, None, ""), Decision::Allow));
        assert!(matches!(g.decide("apply_patch", false, None, ""), Decision::Allow));
        assert!(matches!(g.decide("write_file", false, None, ""), Decision::Allow));

        // safe non-readonly shell commands auto-run in auto
        assert!(matches!(
            g.decide("bash", false, Some("cargo test"), ""),
            Decision::Allow
        ));
        assert!(matches!(
            g.decide("bash", false, Some("git add ."), ""),
            Decision::Allow
        ));
        assert!(matches!(
            g.decide("bash", false, Some("npm run build"), ""),
            Decision::Allow
        ));
        assert!(matches!(
            g.decide("bash", false, Some("mkdir demo"), ""),
            Decision::Allow
        ));

        // destructive shell commands escalate to Ask even in auto
        assert!(matches!(
            g.decide("bash", false, Some("rm -rf target"), ""),
            Decision::Ask { .. }
        ));
        assert!(matches!(
            g.decide("bash", false, Some("git reset --hard HEAD~1"), ""),
            Decision::Ask { .. }
        ));
        assert!(matches!(
            g.decide("bash", false, Some("sudo rm -f secret.txt"), ""),
            Decision::Ask { .. }
        ));
    }

    #[test]
    fn destructive_command_no_false_positives() {
        assert!(!is_destructive("git add ."));
        assert!(!is_destructive("git add src/main.rs"));
        assert!(!is_destructive("cargo test --format json"));
        assert!(!is_destructive("npm run format"));
        assert!(!is_destructive("echo middle"));
        assert!(!is_destructive("cat address.txt"));

        assert!(is_destructive("rm file.txt"));
        assert!(is_destructive("rm -rf node_modules"));
        assert!(is_destructive("/bin/rm -f test"));
        assert!(is_destructive("sudo rm -rf /"));
        assert!(is_destructive("kill -9 1234"));
        assert!(is_destructive("pkill firefox"));
        assert!(is_destructive("git reset --hard"));
        assert!(is_destructive("git clean -f"));
        assert!(is_destructive("echo hi && rm -rf bad"));
    }

    #[test]
    fn accept_edits_allows_readonly_shell() {
        let g = gate("acceptEdits");
        assert!(matches!(
            g.decide("bash", false, Some("ls -la"), ""),
            Decision::Allow
        ));
        assert!(matches!(
            g.decide("bash", false, Some("git status"), ""),
            Decision::Allow
        ));
        assert!(matches!(
            g.decide("bash", false, Some("cat file.txt"), ""),
            Decision::Allow
        ));
    }
}
