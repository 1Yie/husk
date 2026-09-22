//! `steering.rs` — mixed-initiative control.
//!
//! `UiCommand::Steer` injects a user message mid-turn: the state machine is
//! patched in place, so completed tool calls stay valid and `Reasoning` covers
//! the re-plan. The channel plumbing (`steer_tx`/`steer_rx`, drained between tool
//! calls) lives in engine/session; this module owns the probes and the
//! `steered` provenance flag.


use std::time::{Duration, Instant};

/// Minimum interval between ambient suggestions (1 per 5 min).
const PROBE_COOLDOWN: Duration = Duration::from_secs(300);
/// How long a suggestion stays on the status bar.
const SUGGESTION_TTL: Duration = Duration::from_secs(60);

/// Ambient probe — watches the workspace for user-driven build/test errors
/// and surfaces a *suggestion* (never an auto-start).
///
/// Opt-in per repo (`ambient_probe: true` in repo config); the watcher is
/// `notify`-backed and only fires on file-write events that look like a
/// build/test just ran (Cargo.toml/target mtimes, test-runner output).
pub struct AmbientProbe {
    /// Last suggestion instant — cooldown gate.
    last_suggestion: Option<Instant>,
    /// When the current suggestion expires.
    suggestion_deadline: Option<Instant>,
    /// The live suggestion text (None = expired/absent).
    pub pending_suggestion: Option<String>,
    /// Whether the repo opted in.
    enabled: bool,
}

impl AmbientProbe {
    pub fn new(enabled: bool) -> Self {
        Self {
            last_suggestion: None,
            suggestion_deadline: None,
            pending_suggestion: None,
            enabled,
        }
    }

    /// Feed a workspace event (file change that looks build/test-related).
    /// `error_count` comes from a language-server diagnostics poll or a
    /// test-runner failure distill — the probe only decides *whether* to
    /// surface the suggestion.
    pub fn on_build_errors(&mut self, error_count: usize) -> Option<String> {
        if !self.enabled || error_count == 0 {
            return None;
        }
        // Cooldown: max 1 suggestion per 5 min.
        if self
            .last_suggestion
            .map(|t| t.elapsed() < PROBE_COOLDOWN)
            .unwrap_or(false)
        {
            return None;
        }
        let text = format!("{error_count} compile error(s) detected — fix now?");
        self.last_suggestion = Some(Instant::now());
        self.suggestion_deadline = Some(Instant::now() + SUGGESTION_TTL);
        self.pending_suggestion = Some(text.clone());
        Some(text)
    }

    /// Current suggestion, honoring the 60 s TTL — `None` once expired.
    pub fn active_suggestion(&mut self) -> Option<&str> {
        if let Some(dl) = self.suggestion_deadline {
            if Instant::now() > dl {
                self.pending_suggestion = None;
                self.suggestion_deadline = None;
            }
        }
        self.pending_suggestion.as_deref()
    }

    /// Accept the suggestion → it becomes a normal `Prompt`.
    pub fn accept(&mut self) -> Option<String> {
        self.pending_suggestion.take().map(|s| {
            // Turn the suggestion into an actionable prompt.
            format!("Fix the compile errors that just appeared: {s}")
        })
    }
}

/// Provenance flag for a turn that received mid-turn steering — UI shows a
/// `steered` marker on the step capsule.
#[derive(Debug, Default)]
pub struct SteerMark {
    pub steered: bool,
    pub texts: Vec<String>,
}

impl SteerMark {
    pub fn record(&mut self, text: impl Into<String>) {
        self.steered = true;
        self.texts.push(text.into());
    }
}
