//! pulldown-cmark → ratatui `Line`s. Headings bold+accent, fenced code
//! blocks get a `│` gutter, lists get bullets, inline code a dim bg, quotes
//! a `▌` bar, tables an aligned `│ col │ col` grid (widths measured on the
//! plain text of each cell — styling inside cells is dropped to keep the
//! grid honest).

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

struct Ctx {
    lines: Vec<Line<'static>>,
    cur: Vec<Span<'static>>,
    style_stack: Vec<Style>,
    in_code_block: bool,
    list_depth: usize,
    ordered_idx: Option<u64>,
    in_quote: bool,
    in_heading: bool,
    /// Accumulating table — cells land as plain text (in-cell styling is
    /// intentionally dropped; grid alignment depends on width-accurate
    /// content). `None` outside a table.
    table: Option<TableAcc>,
}

/// Row cells for one table under construction.
struct TableAcc {
    rows: Vec<Vec<String>>,
    cur_row: Vec<String>,
    cur_cell: String,
    head_done: bool,
}

fn push_cur(ctx: &mut Ctx) {
    if !ctx.cur.is_empty() || ctx.in_code_block {
        let spans = std::mem::take(&mut ctx.cur);
        let mut spans = spans;
        if ctx.in_code_block {
            spans.insert(0, Span::styled("│ ", Style::default().fg(Color::DarkGray)));
        } else if ctx.in_quote {
            spans.insert(0, Span::styled("▌ ", Style::default().fg(Color::DarkGray)));
        }
        ctx.lines.push(Line::from(spans));
    } else if ctx
        .lines
        .last()
        .map(|l| !l.spans.is_empty())
        .unwrap_or(false)
    {
        // Blank separator between blocks.
        ctx.lines.push(Line::default());
    }
}

fn style(ctx: &Ctx) -> Style {
    ctx.style_stack.last().copied().unwrap_or_default()
}

/// Render a markdown document into styled lines (unwrapped — the caller
/// wraps to terminal width so scrollback lines reflow only at commit time).
pub fn render(md: &str) -> Vec<Line<'static>> {
    let mut ctx = Ctx {
        lines: Vec::new(),
        cur: Vec::new(),
        style_stack: Vec::new(),
        in_code_block: false,
        list_depth: 0,
        ordered_idx: None,
        in_quote: false,
        in_heading: false,
        table: None,
    };
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_TABLES);
    let parser = Parser::new_ext(md, opts);
    for ev in parser {
        match ev {
            Event::Start(tag) => match tag {
                Tag::CodeBlock(_) => ctx.in_code_block = true,
                Tag::Heading { .. } => {
                    ctx.in_heading = true;
                    ctx.style_stack
                        .push(style(&ctx).add_modifier(Modifier::BOLD).fg(Color::Cyan));
                }
                Tag::Strong => ctx
                    .style_stack
                    .push(style(&ctx).add_modifier(Modifier::BOLD)),
                Tag::Emphasis => ctx
                    .style_stack
                    .push(style(&ctx).add_modifier(Modifier::ITALIC)),
                Tag::Strikethrough => ctx
                    .style_stack
                    .push(style(&ctx).add_modifier(Modifier::CROSSED_OUT)),
                Tag::BlockQuote(_) => ctx.in_quote = true,
                Tag::List(start) => {
                    ctx.list_depth += 1;
                    ctx.ordered_idx = start;
                }
                Tag::Item => {
                    push_cur(&mut ctx);
                    let indent = "  ".repeat(ctx.list_depth.saturating_sub(1));
                    let bullet = match ctx.ordered_idx {
                        Some(n) => format!("{indent}{n}. "),
                        None => format!("{indent}• "),
                    };
                    if let Some(n) = ctx.ordered_idx.as_mut() {
                        *n += 1;
                    }
                    ctx.cur
                        .push(Span::styled(bullet, Style::default().fg(Color::DarkGray)));
                }
                Tag::Table(_) => {
                    push_cur(&mut ctx);
                    ctx.table = Some(TableAcc {
                        rows: Vec::new(),
                        cur_row: Vec::new(),
                        cur_cell: String::new(),
                        head_done: false,
                    });
                }
                Tag::TableHead => {}
                Tag::TableRow => {}
                Tag::TableCell => {
                    if let Some(t) = &mut ctx.table {
                        t.cur_cell.clear();
                    }
                }
                Tag::Link { .. } => ctx.style_stack.push(
                    style(&ctx)
                        .fg(Color::Blue)
                        .add_modifier(Modifier::UNDERLINED),
                ),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::CodeBlock => ctx.in_code_block = false,
                TagEnd::Heading(_) => {
                    ctx.in_heading = false;
                    ctx.style_stack.pop();
                    push_cur(&mut ctx);
                }
                TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough | TagEnd::Link => {
                    ctx.style_stack.pop();
                }
                TagEnd::BlockQuote(_) => ctx.in_quote = false,
                TagEnd::Table => {
                    if let Some(t) = ctx.table.take() {
                        emit_table(&mut ctx, t);
                    }
                }
                TagEnd::TableHead => {
                    if let Some(t) = &mut ctx.table {
                        let row = std::mem::take(&mut t.cur_row);
                        t.rows.push(row);
                        t.head_done = true;
                    }
                }
                TagEnd::TableRow => {
                    if let Some(t) = &mut ctx.table {
                        let row = std::mem::take(&mut t.cur_row);
                        t.rows.push(row);
                    }
                }
                TagEnd::TableCell => {
                    if let Some(t) = &mut ctx.table {
                        let cell = std::mem::take(&mut t.cur_cell);
                        t.cur_row.push(cell.trim().to_string());
                    }
                }
                TagEnd::List(_) => {
                    ctx.list_depth = ctx.list_depth.saturating_sub(1);
                    if ctx.list_depth == 0 {
                        ctx.ordered_idx = None;
                    }
                }
                TagEnd::Paragraph => push_cur(&mut ctx),
                _ => {}
            },
            Event::Text(t) => {
                if let Some(tb) = &mut ctx.table {
                    tb.cur_cell.push_str(&t);
                    continue;
                }
                let s = if ctx.in_code_block {
                    Style::default().fg(Color::Yellow)
                } else {
                    style(&ctx)
                };
                ctx.cur.push(Span::styled(t.to_string(), s));
            }
            Event::Code(t) => {
                if let Some(tb) = &mut ctx.table {
                    tb.cur_cell.push_str(&t);
                    continue;
                }
                ctx.cur.push(Span::styled(
                    format!(" {t} "),
                    style(&ctx).bg(Color::DarkGray).fg(Color::White),
                ));
            }
            Event::SoftBreak => {
                if let Some(tb) = &mut ctx.table {
                    tb.cur_cell.push(' ');
                    continue;
                }
                if ctx.in_code_block {
                    push_cur(&mut ctx);
                } else {
                    ctx.cur.push(Span::raw(" "));
                }
            }
            Event::HardBreak => push_cur(&mut ctx),
            Event::Rule => {
                push_cur(&mut ctx);
                ctx.lines.push(Line::from(Span::styled(
                    "─".repeat(20),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            Event::TaskListMarker(checked) => {
                ctx.cur.push(Span::styled(
                    if checked { "[x] " } else { "[ ] " },
                    Style::default().fg(Color::DarkGray),
                ));
            }
            Event::Html(t) | Event::InlineHtml(t) => {
                ctx.cur.push(Span::styled(
                    t.to_string(),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            _ => {}
        }
    }
    push_cur(&mut ctx);
    ctx.lines
}

/// `a │ b │ c` grid with a `──┼──` separator under the header. Widths are
/// per-column max measured on plain cell text — alignment markup is not
/// emitted (terminal tables read fine left-aligned).
fn emit_table(ctx: &mut Ctx, t: TableAcc) {
    if t.rows.is_empty() {
        return;
    }
    let cols = t.rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut widths = vec![0usize; cols];
    for r in &t.rows {
        for (i, c) in r.iter().enumerate() {
            widths[i] = widths[i].max(unicode_width::UnicodeWidthStr::width(c.as_str()));
        }
    }
    let dim = Style::default().fg(Color::DarkGray);
    for (ri, r) in t.rows.iter().enumerate() {
        let mut spans = vec![Span::styled("│ ", dim)];
        for i in 0..cols {
            let cell = r.get(i).cloned().unwrap_or_default();
            let pad =
                widths[i].saturating_sub(unicode_width::UnicodeWidthStr::width(cell.as_str()));
            let st = if ri == 0 && t.head_done {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            spans.push(Span::styled(format!("{cell}{}", " ".repeat(pad)), st));
            spans.push(Span::styled(" │ ", dim));
        }
        ctx.lines.push(Line::from(spans));
        if ri == 0 && t.head_done {
            let sep: String = widths
                .iter()
                .map(|w| "─".repeat(*w))
                .collect::<Vec<_>>()
                .join("─┼─");
            ctx.lines
                .push(Line::from(Span::styled(format!("├─{sep}─┤"), dim)));
        }
    }
    ctx.lines.push(Line::default());
}
