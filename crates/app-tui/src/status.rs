//! Spinner frames + the bottom status strip (workspace dir, git state,
//! turn elapsed, token share) — the info bar under the composer box.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// (branch-or-detached, dirty-file-count) or None when not a git repo.
/// Two short `git` calls, refreshed on a 2s cache — cheap enough to run on
/// the render tick and honest when the repo is absent (empty bar).
pub fn git_info(workspace: &Path) -> Option<(String, usize)> {
    let out = Command::new("git")
        .args(["-C"])
        .arg(workspace)
        .args(["status", "--porcelain=v1", "--branch"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut branch = String::new();
    let mut dirty = 0usize;
    for (i, l) in text.lines().enumerate() {
        if i == 0 {
            // "## main" / "## main...origin/main [ahead 2]" / "## HEAD (no branch)"
            branch = l
                .trim_start_matches('#')
                .trim()
                .split("...")
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
        } else if !l.trim().is_empty() {
            dirty += 1;
        }
    }
    if branch.is_empty() {
        return None;
    }
    Some((branch, dirty))
}

fn dim(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::default().fg(Color::DarkGray))
}

fn fmt_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {}s", s / 60, s % 60)
    } else {
        format!("{}h {}m", s / 3600, (s % 3600) / 60)
    }
}

/// `  agent-rs   main +3  ·  working 2m 14s  ·  ctx 4%` — workspace, git,
/// turn clock, token share. Renders under the composer box.
pub fn bottom_bar(
    workspace: &Path,
    git: Option<&(String, usize)>,
    turn_started: Option<Instant>,
    usage: Option<(u32, u32, u32)>,
) -> Line<'static> {
    let mut spans: Vec<Span> = vec![Span::raw(" ")];
    if let Some(dir) = workspace.file_name().and_then(|n| n.to_str()) {
        spans.push(Span::styled(
            format!(" {dir}"),
            Style::default().add_modifier(Modifier::BOLD),
        ));
    }
    if let Some((branch, dirty)) = git {
        spans.push(dim("   "));
        spans.push(Span::styled(
            branch.clone(),
            Style::default().fg(Color::Magenta),
        ));
        if *dirty > 0 {
            spans.push(dim(format!(" +{dirty}")));
        }
    }
    if let Some(t0) = turn_started {
        spans.push(dim("   ·  "));
        spans.push(Span::styled(
            format!("working {}", fmt_elapsed(t0.elapsed())),
            Style::default().fg(Color::Cyan),
        ));
    }
    // ctx% intentionally absent — the composer border carries it already.
    let _ = usage;
    Line::from(spans)
}
