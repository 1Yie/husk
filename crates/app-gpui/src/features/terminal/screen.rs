//! The terminal protocol owns cursor movement, wrapping, history and selection.
//! The UI consumes a viewport projection; it never parses escape sequences.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

mod handler;

use alacritty_terminal::{
    Term,
    event::{Event, EventListener, WindowSize},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionType},
    term::{Config, TermMode, cell::Flags, color::Colors},
    vte::ansi::{Color, CursorShape, NamedColor, Processor, Rgb},
};

pub const BACKGROUND: u32 = 0x11151a;
pub const FOREGROUND: u32 = 0xd8dee9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalPalette {
    pub foreground: u32,
    pub background: u32,
    pub ansi: Option<[u32; 16]>,
    pub cursor: Option<u32>,
    pub selection: Option<u32>,
}

impl TerminalPalette {
    pub fn scheme(id: &str, dark: bool) -> Self {
        match id {
            "husk" => {
                if dark {
                    Self {
                        foreground: 0xe4e4e7,
                        background: 0x16161a,
                        cursor: Some(0x4da3ff),
                        selection: Some(0x4da3ff40),
                        ansi: Some([
                            0x484f58, 0xff7b72, 0x3fb950, 0xd29922, 0x58a6ff, 0xbc8cff, 0x39c5cf,
                            0xb1bac4, 0x6e7681, 0xffa198, 0x56d364, 0xe3b341, 0x79c0ff, 0xd2a8ff,
                            0x56d4dd, 0xffffff,
                        ]),
                    }
                } else {
                    Self {
                        foreground: 0x1a1c1f,
                        background: 0xffffff,
                        cursor: Some(0x339cff),
                        selection: Some(0x339cff33),
                        ansi: Some([
                            0x24292f, 0xcf222e, 0x116329, 0x4d2d00, 0x0969da, 0x8250df, 0x1b7c83,
                            0x6e7781, 0x57606a, 0xa40e26, 0x1a7f37, 0x633c01, 0x218bff, 0xa475f9,
                            0x3192aa, 0x8c959f,
                        ]),
                    }
                }
            }
            "github-light" => Self {
                foreground: 0x1f2328,
                background: 0xffffff,
                cursor: Some(0x0969da),
                selection: Some(0x0969da33),
                ansi: Some([
                    0x24292f, 0xcf222e, 0x116329, 0x4d2d00, 0x0969da, 0x8250df, 0x1b7c83, 0x6e7781,
                    0x57606a, 0xa40e26, 0x1a7f37, 0x633c01, 0x218bff, 0xa475f9, 0x3192aa, 0x8c959f,
                ]),
            },
            "github-dark" => Self {
                foreground: 0xe6edf3,
                background: 0x0d1117,
                cursor: Some(0x2f81f7),
                selection: Some(0x2f81f740),
                ansi: Some([
                    0x484f58, 0xff7b72, 0x3fb950, 0xd29922, 0x58a6ff, 0xbc8cff, 0x39c5cf, 0xb1bac4,
                    0x6e7681, 0xffa198, 0x56d364, 0xe3b341, 0x79c0ff, 0xd2a8ff, 0x56d4dd, 0xffffff,
                ]),
            },
            "one-dark" => Self {
                foreground: 0xabb2bf,
                background: 0x282c34,
                cursor: Some(0x528bff),
                selection: Some(0x3e4451),
                ansi: Some([
                    0x3f4451, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xd7dae0,
                    0x5c6370, 0xea858b, 0xaad581, 0xffd885, 0x85c0fc, 0xd398eb, 0x6ed5de, 0xffffff,
                ]),
            },
            "solarized-light" => Self {
                foreground: 0x657b83,
                background: 0xfdf6e3,
                cursor: Some(0x586e75),
                selection: Some(0xeee8d5),
                ansi: Some([
                    0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0x93a1a1,
                    0x002b36, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xeee8d5,
                ]),
            },
            "dracula" => Self {
                foreground: 0xf8f8f2,
                background: 0x282a36,
                cursor: Some(0xf8f8f2),
                selection: Some(0x44475a),
                ansi: Some([
                    0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2,
                    0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
                ]),
            },
            "nord" => Self {
                foreground: 0xd8dee9,
                background: 0x2e3440,
                cursor: Some(0xd8dee9),
                selection: Some(0x434c5e),
                ansi: Some([
                    0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0,
                    0x4c566a, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4,
                ]),
            },
            _ => {
                if dark {
                    Self::DARK
                } else {
                    Self::LIGHT
                }
            }
        }
    }

    pub const DARK: Self = Self {
        foreground: FOREGROUND,
        background: BACKGROUND,
        ansi: None,
        cursor: None,
        selection: None,
    };
    pub const LIGHT: Self = Self {
        foreground: 0x303036,
        background: 0xffffff,
        ansi: None,
        cursor: None,
        selection: None,
    };
}

impl Default for TerminalPalette {
    fn default() -> Self {
        Self::DARK
    }
}

#[derive(Clone, Debug)]
pub struct ScreenCell {
    pub row: usize,
    pub column: usize,
    pub text: String,
    /// Zero is a wide-character spacer; the leading cell owns the glyph.
    pub width: usize,
    pub foreground: u32,
    pub background: u32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub selected: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ScreenCursor {
    pub row: usize,
    pub column: usize,
    pub width: usize,
    pub shape: CursorShape,
    pub blinking: bool,
    pub color: u32,
}

#[derive(Clone, Debug)]
pub struct ScreenSnapshot {
    pub foreground: u32,
    pub background: u32,
    pub cells: Vec<ScreenCell>,
    pub cursor: Option<ScreenCursor>,
}

struct Size {
    columns: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

#[derive(Clone, Default)]
struct Listener(Rc<ReplyEvents>);

#[derive(Default)]
struct ReplyEvents {
    events: RefCell<Vec<Event>>,
    /// Only the latest title in a batch matters; do not queue unbounded OSCs.
    title: RefCell<Option<Option<String>>>,
    /// Only populated while forwarding a dynamic_color_sequence callback.
    query_color: Cell<Option<Rgb>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        // Resolve inside the Handler callback, before the parser can apply another
        // color change (even another pair in the same OSC 4). Keep Alacritty's
        // formatter and the shared event queue, so replies retain protocol order.
        let event = match event {
            Event::ColorRequest(_, format) => Event::PtyWrite(format(
                self.0
                    .query_color
                    .get()
                    .expect("color requests must pass through the screen handler"),
            )),
            event => event,
        };
        // Untrusted output cannot read or overwrite the system clipboard.
        match event {
            Event::Title(title) => *self.0.title.borrow_mut() = Some(safe_title(&title)),
            Event::ResetTitle => *self.0.title.borrow_mut() = Some(None),
            Event::PtyWrite(_) | Event::TextAreaSizeRequest(..) => {
                self.0.events.borrow_mut().push(event);
            }
            _ => {}
        }
    }
}

fn safe_title(title: &str) -> Option<String> {
    let title: String = title
        .chars()
        .filter(|character| {
            !character.is_control()
                && !matches!(
                    *character,
                    '\u{061c}' | '\u{200e}'..='\u{200f}' | '\u{202a}'..='\u{202e}'
                        | '\u{2066}'..='\u{2069}'
                )
        })
        .take(80)
        .collect();
    let title = title.trim();
    (!title.is_empty()).then(|| title.to_owned())
}

pub struct TerminalScreen {
    term: Term<Listener>,
    parser: Processor,
    listener: Listener,
    palette: TerminalPalette,
}

impl TerminalScreen {
    pub fn new(columns: usize, rows: usize) -> Self {
        let listener = Listener::default();
        let term = Term::new(
            Config::default(),
            &dimensions(columns, rows),
            listener.clone(),
        );
        Self {
            term,
            parser: Processor::new(),
            listener,
            palette: TerminalPalette::default(),
        }
    }

    /// Changes fallback colors without clearing application-owned OSC overrides.
    pub fn set_palette(&mut self, palette: TerminalPalette) {
        self.palette = palette;
    }

    /// Updates terminal defaults without resetting parser, grid, history or OSC state.
    pub fn apply_preferences(&mut self, prefs: &super::TerminalPreferences) {
        let mut options = Config::default();
        options.default_cursor_style = alacritty_terminal::vte::ansi::CursorStyle {
            shape: match prefs.cursor_style.as_str() {
                "block" => CursorShape::Block,
                "underline" => CursorShape::Underline,
                _ => CursorShape::Beam,
            },
            blinking: prefs.cursor_blink,
        };
        self.term.set_options(options);
    }

    pub fn cursor_blinks(&self) -> bool {
        self.term.cursor_style().blinking
    }

    pub fn palette(&self) -> TerminalPalette {
        self.palette
    }

    /// Resolved defaults include OSC 10/11 overrides until the application resets them.
    pub fn resolved_palette(&self) -> TerminalPalette {
        TerminalPalette {
            foreground: indexed_color(
                NamedColor::Foreground as usize,
                self.term.colors(),
                self.palette,
            ),
            background: indexed_color(
                NamedColor::Background as usize,
                self.term.colors(),
                self.palette,
            ),
            ..self.palette
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(
            &mut handler::ScreenHandler {
                term: &mut self.term,
                listener: &self.listener,
                palette: self.palette,
            },
            bytes,
        );
    }

    pub fn resize(&mut self, columns: usize, rows: usize) {
        self.term.resize(dimensions(columns, rows));
    }

    pub fn dimensions(&self) -> (usize, usize) {
        (self.term.columns(), self.term.screen_lines())
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    pub fn scroll(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    fn point(&self, row: usize, column: usize) -> Point {
        Point::new(
            Line(
                row.min(self.term.screen_lines() - 1) as i32
                    - self.term.grid().display_offset() as i32,
            ),
            Column(column.min(self.term.columns() - 1)),
        )
    }

    pub fn begin_selection(&mut self, row: usize, column: usize, right: bool, clicks: usize) {
        let kind = match clicks {
            2 => SelectionType::Semantic,
            3.. => SelectionType::Lines,
            _ => SelectionType::Simple,
        };
        self.term.selection = Some(Selection::new(
            kind,
            self.point(row, column),
            if right { Side::Right } else { Side::Left },
        ));
    }

    pub fn extend_selection(&mut self, row: usize, column: usize, right: bool) {
        let point = self.point(row, column);
        if let Some(selection) = &mut self.term.selection {
            selection.update(point, if right { Side::Right } else { Side::Left });
        }
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    pub fn selected_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }

    /// Protocol replies are sent through the same serialized PTY writer as keys.
    pub fn take_replies(&self, cell_width: u16, cell_height: u16) -> Vec<String> {
        self.listener
            .0
            .events
            .borrow_mut()
            .drain(..)
            .filter_map(|event| match event {
                Event::PtyWrite(text) => Some(text),
                Event::TextAreaSizeRequest(format) => Some(format(WindowSize {
                    num_lines: self.term.screen_lines() as u16,
                    num_cols: self.term.columns() as u16,
                    cell_width,
                    cell_height,
                })),
                _ => None,
            })
            .collect()
    }

    /// A title change is independent of PTY replies; `Some(None)` means reset.
    pub fn take_title(&self) -> Option<Option<String>> {
        self.listener.0.title.borrow_mut().take()
    }

    pub fn snapshot(&self) -> ScreenSnapshot {
        let content = self.term.renderable_content();
        let columns = self.term.columns();
        let rows = self.term.screen_lines();
        let mut cells = Vec::with_capacity(columns * rows);
        for indexed in content.display_iter {
            let row = indexed.point.line.0 + content.display_offset as i32;
            if row < 0 || row >= rows as i32 {
                continue;
            }
            let cell = indexed.cell;
            let flags = cell.flags;
            let spacer =
                flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER);
            let mut foreground = color(
                cell.fg,
                content.colors,
                flags.contains(Flags::BOLD),
                self.palette,
            );
            let mut background = color(cell.bg, content.colors, false, self.palette);
            if flags.contains(Flags::DIM) {
                foreground = dim(foreground);
            }
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut foreground, &mut background);
            }
            let mut text = String::new();
            if !spacer && !flags.contains(Flags::HIDDEN) {
                text.push(cell.c);
                if let Some(extra) = cell.zerowidth() {
                    text.extend(extra);
                }
            }
            cells.push(ScreenCell {
                row: row as usize,
                column: indexed.point.column.0,
                text,
                width: if spacer {
                    0
                } else if flags.contains(Flags::WIDE_CHAR) {
                    2
                } else {
                    1
                },
                foreground,
                background,
                bold: flags.contains(Flags::BOLD),
                italic: flags.contains(Flags::ITALIC),
                underline: flags.intersects(Flags::ALL_UNDERLINES),
                strikethrough: flags.contains(Flags::STRIKEOUT),
                selected: content.selection.is_some_and(|selection| {
                    // Copying either half includes the whole wide glyph. Keep
                    // both physical cell backgrounds and glyph color in sync.
                    // The panel paints its cursor separately from selection.
                    selection.contains_cell(&indexed, content.cursor.point, CursorShape::Hidden)
                        || (flags.contains(Flags::WIDE_CHAR_SPACER)
                            && indexed.point.column.0 > 0
                            && selection
                                .contains(Point::new(indexed.point.line, indexed.point.column - 1)))
                }),
            });
        }
        let row = content.cursor.point.line.0 + content.display_offset as i32;
        let cursor = (content.cursor.shape != CursorShape::Hidden && row >= 0 && row < rows as i32)
            .then(|| ScreenCursor {
                row: row as usize,
                column: content.cursor.point.column.0,
                width: if self.term.grid()[content.cursor.point]
                    .flags
                    .contains(Flags::WIDE_CHAR)
                {
                    2
                } else {
                    1
                },
                shape: content.cursor.shape,
                blinking: self.term.cursor_style().blinking,
                color: indexed_color(NamedColor::Cursor as usize, content.colors, self.palette),
            });
        ScreenSnapshot {
            foreground: indexed_color(
                NamedColor::Foreground as usize,
                content.colors,
                self.palette,
            ),
            background: indexed_color(
                NamedColor::Background as usize,
                content.colors,
                self.palette,
            ),
            cells,
            cursor,
        }
    }
}

fn dimensions(columns: usize, rows: usize) -> Size {
    let columns = columns.clamp(2, 1000);
    Size {
        columns,
        rows: rows.clamp(1, 1000).min(250_000 / columns),
    }
}

fn color(color: Color, colors: &Colors, bold: bool, palette: TerminalPalette) -> u32 {
    match color {
        Color::Spec(rgb) => (rgb.r as u32) << 16 | (rgb.g as u32) << 8 | rgb.b as u32,
        Color::Named(named) => indexed_color(
            if bold && (named as usize) < 8 {
                named as usize + 8
            } else {
                named as usize
            },
            colors,
            palette,
        ),
        Color::Indexed(index) => indexed_color(
            if bold && index < 8 {
                index as usize + 8
            } else {
                index as usize
            },
            colors,
            palette,
        ),
    }
}

fn indexed_color(index: usize, colors: &Colors, palette: TerminalPalette) -> u32 {
    if let Some(rgb) = colors[index] {
        return (rgb.r as u32) << 16 | (rgb.g as u32) << 8 | rgb.b as u32;
    }
    const ANSI: [u32; 16] = [
        0x1d232a, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xd8dee9, 0x5c6370,
        0xff7a85, 0xb1db91, 0xffd68c, 0x80c7ff, 0xe49af5, 0x7dd8e3, 0xffffff,
    ];
    const LIGHT_ANSI: [u32; 16] = [
        0x24292f, 0xcf222e, 0x116329, 0x4d2d00, 0x0969da, 0x8250df, 0x1b7c83, 0x6e7781, 0x57606a,
        0xa40e26, 0x1a7f37, 0x633c01, 0x218bff, 0xa475f9, 0x3192aa, 0x8c959f,
    ];
    let brightness = ((palette.background >> 16) & 255)
        + ((palette.background >> 8) & 255)
        + (palette.background & 255);
    let ansi = if brightness > 384 { &LIGHT_ANSI } else { &ANSI };
    match index {
        0..=15 => palette.ansi.as_ref().unwrap_or(ansi)[index],
        16..=231 => {
            let value = index - 16;
            let component = |n: usize| if n == 0 { 0 } else { 55 + 40 * n as u32 };
            component(value / 36) << 16 | component(value / 6 % 6) << 8 | component(value % 6)
        }
        232..=255 => {
            let value = 8 + (index as u32 - 232) * 10;
            value * 0x010101
        }
        257 => palette.background,
        // Preserve the native OSC-10 cursor fallback; OSC-12 above wins first.
        258 if colors[NamedColor::Foreground as usize].is_some() => {
            indexed_color(NamedColor::Foreground as usize, colors, palette)
        }
        258 => palette
            .cursor
            .unwrap_or_else(|| indexed_color(NamedColor::Foreground as usize, colors, palette)),
        259..=266 => dim(palette.ansi.as_ref().unwrap_or(ansi)[index - 259]),
        268 => dim(palette.foreground),
        _ => palette.foreground,
    }
}

fn dim(rgb: u32) -> u32 {
    let component = |shift: u32| (((rgb >> shift) & 255_u32) as f32 * 0.66) as u32;
    component(16) << 16 | component(8) << 8 | component(0)
}

#[cfg(test)]
mod tests {
    use super::dimensions;

    #[test]
    fn dimensions_remain_within_vt_minimums_and_pty_limits() {
        for (requested, expected) in [
            ((0, 0), (2, 1)),
            ((1, 1), (2, 1)),
            ((80, 24), (80, 24)),
            ((usize::MAX, 1), (1000, 1)),
            ((2, usize::MAX), (2, 1000)),
            ((usize::MAX, usize::MAX), (1000, 250)),
        ] {
            let size = dimensions(requested.0, requested.1);
            assert_eq!((size.columns, size.rows), expected);
            assert!(size.columns * size.rows <= 250_000);
        }
    }
}
