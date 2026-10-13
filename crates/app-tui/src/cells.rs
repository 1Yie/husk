//! Transcript cells — the display-side mirror of `UiEvent`. Committed
//! cells render once into scrollback lines; the in-flight stream lives
//! in `App::live` until the turn commits it.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::markdown;

fn dim(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::default().fg(Color::DarkGray))
}

/// One committed transcript entry. `to_lines` produces styled, UNWRAPPED
/// lines — wrapping happens at commit width in `transcript::wrap_lines`.
#[derive(Debug)]
pub enum Cell {
    /// User turn — `❯ text` on a subtle highlight row, `h:MM AM` stamped
    /// at the right edge (pi-style turn boundary).
    User { text: String, time: String },
    /// Assistant reply — markdown body + right-aligned timestamp on the
    /// last line when there's room.
    Agent { md: String, time: String },
    /// `Worked for Ns` — turn-elapsed footer after the reply lands.
    Worked { secs: f32 },
    Tool {
        name: String,
        preview: String,
        ok: bool,
        /// tail lines kept on failure (already plain text)
        tail: Vec<String>,
        /// measured duration — shown as `· 4.1s` when present.
        secs: Option<f32>,
    },
    Sys(String),
    Err(String),
    Note(String), // dim one-liner (whitelist grant, retry marker, compaction…)
    Plan(String),
    /// Model reasoning committed at a tool boundary — dim italic.
    Reasoning(String),
    /// One `batch_execute` child row, indented under its parent.
    Child(crate::app::ChildTool),
    /// Pre-styled lines committed verbatim — help/status cards build
    /// their own spans, the cell is just the carrier.
    Lines(Vec<Line<'static>>),
    /// Startup card: app name + session facts + key hints.
    Welcome {
        model: String,
        sandbox: String,
        workspace: String,
    },
}

impl Cell {
    /// `width` — needed for the user-row highlight fill and right-aligned
    /// timestamps; commit-time width comes from `App::width`.
    pub fn to_lines(&self, width: u16) -> Vec<Line<'static>> {
        match self {
            Cell::User { text, time } => {
                // ` text ······· 5:33 AM` — full-width tinted row.
                let bg = Style::default().bg(Color::Indexed(234));
                let head_w = 1 + unicode_width::UnicodeWidthStr::width(text.as_str());
                let tail_w = unicode_width::UnicodeWidthStr::width(time.as_str()) + 1;
                let pad = (width as usize).saturating_sub(head_w + tail_w);
                let mut out = vec![Line::from(vec![
                    Span::styled(
                        format!(" {text}"),
                        bg.fg(Color::White).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(" ".repeat(pad.max(1)), bg),
                    Span::styled(format!("{time} "), bg.fg(Color::DarkGray)),
                ])];
                out.push(Line::default());
                out
            }
            Cell::Agent { md, time } => {
                let mut out = markdown::render(md);
                // pi-style stamp: right-aligned on the last content line
                // when it fits; a row of its own when it doesn't.
                let tw = unicode_width::UnicodeWidthStr::width(time.as_str()) + 2;
                let stamped = out.last().is_some_and(|l| l.width() + tw <= width as usize);
                if stamped {
                    let last = out.last_mut().unwrap();
                    let pad = (width as usize).saturating_sub(last.width() + tw);
                    last.spans.push(Span::raw(" ".repeat(pad)));
                    last.spans.push(Span::styled(
                        time.clone(),
                        Style::default().fg(Color::DarkGray),
                    ));
                } else if !out.is_empty() {
                    let pad = (width as usize).saturating_sub(tw);
                    out.push(Line::from(vec![
                        Span::raw(" ".repeat(pad)),
                        Span::styled(time.clone(), Style::default().fg(Color::DarkGray)),
                    ]));
                }
                out.push(Line::default());
                out
            }
            Cell::Worked { secs } => vec![
                Line::from(Span::styled(
                    format!(" Worked for {secs:.1}s"),
                    Style::default().fg(Color::DarkGray),
                )),
                Line::default(),
            ],
            Cell::Tool {
                name,
                preview,
                ok,
                tail,
                secs,
            } => {
                let (mark, color) = if *ok {
                    ("✓", Color::Green)
                } else {
                    ("✗", Color::Red)
                };
                let mut spans = vec![Span::styled(
                    format!("{mark} {name}"),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )];
                if !preview.is_empty() {
                    spans.push(dim(format!("  {preview}")));
                }
                if let Some(s) = secs {
                    spans.push(dim(format!("  · {s:.1}s")));
                }
                let mut out = vec![Line::from(spans)];
                for t in tail {
                    out.push(Line::from(dim(format!("    {t}"))));
                }
                out
            }
            Cell::Lines(ls) => {
                let mut out = ls.clone();
                out.push(Line::default());
                out
            }
            Cell::Welcome {
                model,
                sandbox,
                workspace,
            } => vec![
                Line::default(),
                Line::from(vec![
                    Span::styled(
                        " agent",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("-tui", Style::default().add_modifier(Modifier::BOLD)),
                ]),
                Line::from(dim(format!(" {model} · sandbox:{sandbox}"))),
                Line::from(dim(format!(" {workspace}"))),
                Line::from(dim(" /help 命令 · Esc/Ctrl+C 打断 · Ctrl+D 退出")),
                Line::default(),
            ],
            Cell::Sys(m) => vec![Line::from(dim(format!("· {m}")))],
            Cell::Err(e) => vec![Line::from(Span::styled(
                format!("error: {e}"),
                Style::default().fg(Color::Red),
            ))],
            Cell::Note(m) => vec![Line::from(dim(format!("— {m} —")))],
            Cell::Reasoning(text) => {
                let style = Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC);
                let mut out: Vec<Line<'static>> = text
                    .lines()
                    .map(|l| Line::from(Span::styled(format!("  {l}"), style)))
                    .collect();
                out.push(Line::default());
                out
            }
            Cell::Child(child) => {
                let (mark, color) = if !child.done {
                    ("  …", Color::DarkGray)
                } else if child.ok {
                    ("  ✓", Color::Green)
                } else if child.ui_type.as_deref() == Some("rejected") {
                    ("  ⊘", Color::Yellow)
                } else {
                    ("  ✗", Color::Red)
                };
                let mut out = vec![Line::from(vec![
                    Span::styled(format!("{mark} {}", child.name), Style::default().fg(color)),
                    dim(format!("  {}", child.preview)),
                ])];
                for t in &child.tail {
                    out.push(Line::from(dim(format!("      {t}"))));
                }
                out
            }
            Cell::Plan(plan) => {
                // `plan` is a JSON string from the kernel — pretty-print the
                // readable fields, fall back to raw text.
                let pretty = serde_json::from_str::<serde_json::Value>(plan)
                    .map(|v| {
                        let mut s = String::new();
                        for k in ["summary", "steps", "verification", "risks"] {
                            if let Some(val) = v.get(k) {
                                s.push_str(&format!("{k}: {}\n", fmt_val(val)));
                            }
                        }
                        if s.is_empty() {
                            plan.clone()
                        } else {
                            s.trim_end().to_string()
                        }
                    })
                    .unwrap_or_else(|_| plan.clone());
                let mut out = vec![Line::from(Span::styled(
                    "▣ plan",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ))];
                out.extend(
                    pretty
                        .lines()
                        .map(|l| Line::from(Span::raw(format!("  {l}")))),
                );
                out.push(Line::default());
                out
            }
        }
    }
}

fn fmt_val(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(i, v)| format!("{}. {}", i + 1, fmt_val(v)))
            .collect::<Vec<_>>()
            .join("\n   "),
        other => other.to_string(),
    }
}
