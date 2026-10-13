//! Committed transcript → native scrollback.
//!
//! Finished cells are width-wrapped, queued on `App::pending`, then flushed
//! into the terminal's real scrollback via `Terminal::insert_before` above
//! the inline viewport. Scrolling/copy/selection stay terminal-native.
//! Already-printed lines do not reflow on resize (wrap happens at commit
//! width; a reflow pass is the known follow-up).

use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

/// Wrap a styled `Line` to `width` cells, splitting spans at char
/// boundaries and preserving each span's style. Zero-width / empty input
/// still yields one (empty) line so blank separators survive.
pub fn wrap_line(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut cur: Vec<Span<'static>> = Vec::new();
    let mut cur_w = 0usize;

    let mut flush = |cur: &mut Vec<Span<'static>>, cur_w: &mut usize| {
        out.push(Line::from(std::mem::take(cur)));
        *cur_w = 0;
    };

    for span in &line.spans {
        let style = span.style;
        let mut seg = String::new();
        for ch in span.content.chars() {
            let w = UnicodeWidthChar::width(ch).unwrap_or(0);
            if ch == '\n' || cur_w + w > width {
                // hard break inside the span's own text, or wrap point
                if !seg.is_empty() {
                    cur.push(Span::styled(std::mem::take(&mut seg), style));
                }
                flush(&mut cur, &mut cur_w);
                if ch == '\n' {
                    continue;
                }
            }
            seg.push(ch);
            cur_w += w;
        }
        if !seg.is_empty() {
            cur.push(Span::styled(seg, style));
        }
    }
    flush(&mut cur, &mut cur_w);
    out
}

/// A line that ended up wider than expected gets hard-wrapped; the commit
/// path routes every `Line` through `wrap_line` so nothing overflows the
/// scrollback width and corrupts the frame below.
pub fn wrap_lines(lines: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    let w = width.max(8) as usize;
    lines.iter().flat_map(|l| wrap_line(l, w)).collect()
}
