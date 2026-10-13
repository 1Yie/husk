//! Viewport rendering — the inline "current frame": activity region +
//! bordered composer/modal pane with status folded into the border.
//!
//! The input lives inside a rounded box whose bottom border carries the
//! live status (left: spinner+state+flags, right: provider/model + ctx%).
//! No standalone status row inside the box — the border IS the status.

use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::app::App;
use crate::modals::Modal;
use crate::status::SPINNER;

fn dim(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::default().fg(Color::DarkGray))
}

/// Left half of the pane's bottom border: spinner + state + flags.
fn border_left(app: &App) -> Line<'static> {
    let mut spans: Vec<Span> = Vec::new();
    if app.state.is_active() {
        spans.push(Span::styled(
            format!("{} ", SPINNER[app.spinner % SPINNER.len()]),
            Style::default().fg(Color::Cyan),
        ));
        spans.push(Span::styled(
            app.state.label(),
            Style::default().fg(Color::Cyan),
        ));
    } else {
        spans.push(Span::styled(
            app.state.label(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if !app.queued.is_empty() {
        spans.push(dim(format!("  queued:{}", app.queued.len())));
    }
    if !app.sandbox_id.is_empty() {
        let off = app.sandbox_id == "none" || app.sandbox_id == "off";
        spans.push(Span::styled(
            format!("  sandbox:{}", app.sandbox_id),
            if off {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

/// Right half: `provider/model (thinking) · ctx%`.
fn border_right(app: &App) -> Line<'static> {
    // Always show a level — unset = provider default, "(默认)" is honest.
    let level = app.thinking_level.as_deref().unwrap_or("默认");
    let model = format!("{}/{} ({})", app.provider_name, app.model, level);
    let mut s = format!(" {model}");
    if let Some((_, used, window)) = app.usage {
        if window > 0 {
            let pct = (used as f64 / window as f64 * 100.0) as u32;
            s = format!(" {model} · ctx {}%", pct.min(100));
        }
    }
    Line::from(dim(format!("{s} "))).right_aligned()
}

/// Render into the inline viewport `area`.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    let marker_width = area.width.saturating_sub(3).min(3);
    let (composer_rows, composer_cursor) = app
        .composer
        .wrapped(area.width.saturating_sub(2 + marker_width).max(1) as usize);

    // Bottom pane height: modal content + 2 border rows, else composer + 2.
    // The modal gets real room — clamping it to a tiny fixed height clips
    // lists mid-scroll (the "can't see the rest" bug); leave just enough
    // rows above for the activity region and the status strip.
    let pane_rows = if let Some(m) = &app.modal {
        let max = (area.height as usize).saturating_sub(4).max(6);
        m.height(area.width.saturating_sub(4) as usize, max) as u16 + 2
    } else {
        composer_rows.len().min(5) as u16 + 2
    };

    // `/` candidates strip — sits right above the composer box like an
    // autocomplete dropdown; zero height while no prefix is being typed.
    let cands = if app.modal.is_none() {
        app.slash_candidates()
    } else {
        Vec::new()
    };
    // strip = up to 7 candidates + a position row when the list overflows
    let sug_rows = if cands.len() > 8 {
        8
    } else {
        cands.len() as u16
    };
    // Persistent todo strip — up to 4 items + header, parked above the
    // composer until the tool replaces or clears the list.
    let todo_rows = if app.todos.is_empty() {
        0
    } else {
        (app.todos.len().min(4) + 1) as u16
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(sug_rows),
            Constraint::Length(todo_rows),
            Constraint::Length(pane_rows),
            Constraint::Length(1), // bottom status strip
        ])
        .split(area);

    // --- activity region: streaming text + reasoning tail + tool lines ---
    let act = app.activity_lines(area.width as usize, chunks[0].height as usize);
    if !act.is_empty() {
        frame.render_widget(Paragraph::new(act).wrap(Wrap { trim: false }), chunks[0]);
    }

    // --- `/` autocomplete candidates ---
    if sug_rows > 0 {
        // Follow `slash_sel` with a scroll window — a naive take(8) leaves
        // the selected row off-screen once it's past the first page.
        let vis = if cands.len() > 8 { 7 } else { 8 };
        // win_start is tuned for LIST_MAX=9 — scale the window to `vis`.
        let start = if cands.len() <= vis {
            0
        } else if app.slash_sel + 1 >= cands.len() {
            cands.len() - vis
        } else {
            app.slash_sel.saturating_sub(vis - 1)
        };
        let mut sug_lines = Vec::new();
        for (i, (name, desc)) in cands.iter().enumerate().skip(start).take(vis) {
            let on = i == app.slash_sel;
            sug_lines.push(Line::from(vec![
                Span::styled(
                    if on { " ▸ " } else { "   " },
                    Style::default().fg(if on { Color::Cyan } else { Color::DarkGray }),
                ),
                Span::styled(
                    format!("{name:<14}"),
                    Style::default().fg(if on { Color::White } else { Color::Cyan }),
                ),
                Span::styled(*desc, Style::default().fg(Color::DarkGray)),
            ]));
        }
        if cands.len() > 8 {
            sug_lines.push(Line::from(Span::styled(
                format!("   ({}/{})", app.slash_sel + 1, cands.len()),
                Style::default().fg(Color::DarkGray),
            )));
        }
        frame.render_widget(Paragraph::new(sug_lines), chunks[1]);
    }

    // --- todo panel: checkbox rows, done items dimmed+struck ---
    if todo_rows > 0 {
        let done_n = app.todos.iter().filter(|t| t.done).count();
        let total = app.todos.len();
        let mut tl: Vec<Line> = vec![Line::from(vec![
            Span::styled(
                " ◉ ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("todo {done_n}/{total}"),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            if total > 4 {
                Span::styled(
                    format!("   +{} 更多", total - 4),
                    Style::default().fg(Color::DarkGray),
                )
            } else {
                Span::raw("")
            },
        ])];
        for t in app.todos.iter().take(4) {
            // `[x]`/`[ ]` — ASCII boxes keep done/undone rows column-equal
            // (☑/☐ glyphs are different cell widths in most fonts).
            let (mark, st) = if t.done {
                ("[x]", Style::default().fg(Color::Green))
            } else {
                ("[ ]", Style::default().fg(Color::DarkGray))
            };
            let text_style = if t.done {
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::CROSSED_OUT)
            } else {
                Style::default()
            };
            tl.push(Line::from(vec![
                Span::styled(format!("   {mark} "), st),
                Span::styled(
                    format!("#{:<3}", t.id),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(t.text.chars().take(60).collect::<String>(), text_style),
            ]));
        }
        frame.render_widget(Paragraph::new(tl), chunks[2]);
    }

    // --- bottom pane ---
    let pane = chunks[3];
    let t_left = border_left(app);
    let t_right = border_right(app);
    if let Some(m) = &mut app.modal {
        let accent = match m {
            Modal::Approval { .. } => Color::Yellow,
            Modal::Question { .. } => Color::Cyan,
            Modal::Sessions { .. } | Modal::Models { .. } | Modal::Pick { .. } => Color::Cyan,
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(accent))
            .title_bottom(t_left)
            .title_bottom(t_right);
        let inner = block.inner(pane);
        frame.render_widget(block, pane);
        // Paragraph only writes the cells its text touches — the previous
        // frame's composer text would bleed through otherwise.
        frame.render_widget(Clear, inner);
        frame.render_widget(
            Paragraph::new(m.lines(inner.width.max(1) as usize)).wrap(Wrap { trim: false }),
            inner,
        );
    } else {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title_bottom(t_left)
            .title_bottom(t_right);
        let inner = block.inner(pane);
        frame.render_widget(block, pane);
        frame.render_widget(Clear, inner);

        let scroll = composer_cursor
            .0
            .saturating_sub(inner.height.saturating_sub(1) as usize);
        // Text and cursor share the same wrapped rows and scroll offset.
        let lines: Vec<Line> = composer_rows
            .iter()
            .enumerate()
            .skip(scroll)
            .take(inner.height as usize)
            .map(|(i, l)| {
                let marker = if i == 0 && marker_width == 3 {
                    " ❯ ".into()
                } else {
                    " ".repeat(marker_width as usize)
                };
                Line::from(vec![
                    Span::styled(
                        marker,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    if l.is_empty() && i == 0 && app.composer.is_empty() {
                        Span::styled(
                            "消息（Enter 发送 · Ctrl+J 换行 · /help）",
                            Style::default().fg(Color::DarkGray),
                        )
                    } else {
                        Span::raw(l.clone())
                    },
                ])
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);

        // Cursor inside the box: marker width + text col.
        if inner.width > 0 && inner.height > 0 {
            frame.set_cursor_position((
                inner.x + marker_width + composer_cursor.1 as u16,
                inner.y + (composer_cursor.0 - scroll) as u16,
            ));
        }
    }

    // --- bottom status strip: dir · git · turn clock · token share ---
    frame.render_widget(
        Paragraph::new(crate::status::bottom_bar(
            &app.workspace,
            app.git_info.as_ref(),
            app.turn_started,
            app.usage,
        )),
        chunks[4],
    );
}
