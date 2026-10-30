use std::{
    ops::Range,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use alacritty_terminal::{term::TermMode, vte::ansi::CursorShape};
use async_channel::{Receiver, Sender};
use gpui_kit::component::ActiveTheme;
use gpui_kit::{prelude::*, *};
use husk_terminal::{CapacityWait, TerminalEvent, TerminalState};

use super::TerminalPreferences;
use super::screen::{ScreenSnapshot, TerminalPalette, TerminalScreen};

const MAX_PASTE: usize = 1024 * 1024;

enum Command {
    Input(String),
    Resize(usize, usize),
}

enum Update {
    Started(Result<(), String>),
    Pty(TerminalEvent),
    Error(String),
}

enum Phase {
    Starting,
    Running,
    Exited(String),
    Failed(String),
}

/// All registry operations which can wait for I/O or a child run on this worker.
/// Dropping its channel requests cancellation, including during an in-flight open.
struct PtyWorker {
    commands: Sender<Command>,
    cancelled: Arc<AtomicBool>,
}

impl Drop for PtyWorker {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.commands.close();
    }
}

impl PtyWorker {
    fn start(
        registry: TerminalState,
        owner: String,
        cwd: PathBuf,
        updates: Sender<Update>,
    ) -> Result<Self, String> {
        let (commands, receiver) = async_channel::bounded(64);
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        std::thread::Builder::new()
            .name("native-terminal".into())
            .spawn(move || {
                run_pty(registry, owner, cwd, updates, receiver, stop);
            })
            .map_err(|error| format!("Cannot start terminal worker: {error}"))?;
        Ok(Self {
            commands,
            cancelled,
        })
    }

    fn send(&self, command: Command) -> Result<(), String> {
        self.commands.try_send(command).map_err(|error| {
            if error.is_full() {
                "Terminal input queue is full".into()
            } else {
                "Terminal is closed".into()
            }
        })
    }
}

fn run_pty(
    registry: TerminalState,
    owner: String,
    cwd: PathBuf,
    updates: Sender<Update>,
    commands: Receiver<Command>,
    cancelled: Arc<AtomicBool>,
) {
    let sink_updates = updates.clone();
    let sink_stop = cancelled.clone();
    let sink = Arc::new(move |event| {
        if sink_stop.load(Ordering::Acquire) {
            return Err("terminal view was released".into());
        }
        sink_updates
            .send_blocking(Update::Pty(event))
            .map_err(|_| "terminal view was released".into())
    });
    let opened = registry.open_when_available(
        owner.clone(),
        Some(cwd.to_string_lossy().into_owned()),
        80,
        24,
        sink,
        CapacityWait {
            cancelled: &cancelled,
            timeout: Duration::from_secs(2),
        },
    );
    let opened = match opened {
        Ok(opened) => opened,
        Err(error) => {
            let _ = updates.send_blocking(Update::Started(Err(error)));
            return;
        }
    };
    if !cancelled.load(Ordering::Acquire) && updates.send_blocking(Update::Started(Ok(()))).is_ok()
    {
        while let Ok(command) = commands.recv_blocking() {
            if cancelled.load(Ordering::Acquire) {
                break;
            }
            let result = match command {
                Command::Input(text) => {
                    // The service caps individual writes at 64 KiB; keep a
                    // paste atomic with respect to subsequent key commands.
                    let mut remaining = text.as_str();
                    let mut result = Ok(());
                    while !remaining.is_empty() && !cancelled.load(Ordering::Acquire) {
                        let mut end = remaining.len().min(64 * 1024);
                        while !remaining.is_char_boundary(end) {
                            end -= 1;
                        }
                        if let Err(error) = registry.write(&owner, &opened.id, &remaining[..end]) {
                            result = Err(error);
                            break;
                        }
                        remaining = &remaining[end..];
                    }
                    result
                }
                Command::Resize(columns, rows) => {
                    registry.resize(&owner, &opened.id, columns as u32, rows as u32)
                }
            };
            if let Err(error) = result
                && updates.send_blocking(Update::Error(error)).is_err()
            {
                break;
            }
        }
    }
    // close waits for reaping; it must never run on the window thread.
    let _ = registry.close(&owner, &opened.id);
}

pub struct TerminalPanel {
    screen: TerminalScreen,
    preferences: TerminalPreferences,
    fonts: [Font; 4],
    geometry_dirty: bool,
    blink_visible: bool,
    focused: bool,
    last_paint: Instant,
    _preferences: Subscription,
    _blink: Task<()>,
    title: Option<String>,
    focus_handle: FocusHandle,
    worker: Option<PtyWorker>,
    phase: Phase,
    input_error: Option<String>,
    last_resize: Option<(usize, usize)>,
    bounds: Bounds<Pixels>,
    cell_width: Pixels,
    selecting: bool,
    scroll_remainder: f32,
    /// Only composition is editable locally. Committed text belongs to the PTY.
    marked: String,
    marked_selection: Range<usize>,
    marked_layout: Option<ShapedLine>,
    marked_origin: Point<Pixels>,
    _events: Task<()>,
    _focus_events: Vec<Subscription>,
}

pub(super) enum TerminalPanelEvent {
    TitleChanged,
}

impl EventEmitter<TerminalPanelEvent> for TerminalPanel {}

impl TerminalPanel {
    pub fn new(
        registry: TerminalState,
        owner: String,
        cwd: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let preferences = cx
            .try_global::<TerminalPreferences>()
            .cloned()
            .unwrap_or_default();
        let fonts = terminal_fonts(&preferences.font_family);
        let subscription = cx.observe_global::<TerminalPreferences>(|_, cx| cx.notify());
        let blink = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        if this.focused && this.last_paint.elapsed() < Duration::from_secs(1) {
                            this.blink_visible = !this.blink_visible;
                            if this.screen.cursor_blinks() {
                                cx.notify();
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let focus_handle = cx.focus_handle();
        let (tx, rx) = async_channel::bounded(128);
        let worker = PtyWorker::start(registry, owner, cwd, tx);
        let (worker, phase) = match worker {
            Ok(worker) => (Some(worker), Phase::Starting),
            Err(error) => (None, Phase::Failed(error)),
        };
        let events = cx.spawn(async move |this, cx| {
            while let Ok(update) = rx.recv().await {
                if this
                    .update(cx, |this, cx| {
                        let previous_title = this.title.clone();
                        this.update(update);
                        // A bounded batch reduces redraws without starving the window.
                        for _ in 0..127 {
                            let Ok(update) = rx.try_recv() else {
                                break;
                            };
                            this.update(update);
                        }
                        if this.title != previous_title {
                            cx.emit(TerminalPanelEvent::TitleChanged);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let focus_events = vec![
            cx.on_focus(&focus_handle, window, |this, _, cx| {
                if this.screen.mode().contains(TermMode::FOCUS_IN_OUT) {
                    this.send_input("\x1b[I".into());
                }
                cx.notify();
            }),
            cx.on_blur(&focus_handle, window, |this, window, cx| {
                if this.screen.mode().contains(TermMode::FOCUS_IN_OUT) {
                    this.send_input("\x1b[O".into());
                }
                this.focused = false;
                this.marked.clear();
                this.marked_layout = None;
                window.notify_text_input_changed(&this.focus_handle);
                cx.notify();
            }),
        ];
        Self {
            screen: {
                let mut screen = TerminalScreen::new(80, 24);
                screen.apply_preferences(&preferences);
                screen
            },
            preferences,
            fonts,
            geometry_dirty: true,
            blink_visible: true,
            focused: false,
            last_paint: Instant::now(),
            _preferences: subscription,
            _blink: blink,
            title: None,
            focus_handle,
            worker,
            phase,
            input_error: None,
            last_resize: None,
            bounds: Bounds::default(),
            cell_width: px(8.4),
            selecting: false,
            scroll_remainder: 0.,
            marked: String::new(),
            marked_selection: 0..0,
            marked_layout: None,
            marked_origin: point(px(0.), px(0.)),
            _events: events,
            _focus_events: focus_events,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
    }

    pub(super) fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub(super) fn status(&self) -> Option<String> {
        match &self.phase {
            Phase::Starting => Some("Starting terminal…".to_owned()),
            Phase::Failed(error) => Some(format!("Terminal could not start: {error}")),
            Phase::Exited(message) => Some(message.clone()),
            Phase::Running => self.input_error.clone(),
        }
    }

    fn update(&mut self, update: Update) {
        match update {
            Update::Started(Ok(())) => {
                // A short-lived child may have delivered its exit before open returned.
                if matches!(self.phase, Phase::Starting) {
                    self.phase = Phase::Running;
                }
                self.last_resize = None;
            }
            Update::Started(Err(error)) => self.phase = Phase::Failed(error),
            Update::Pty(TerminalEvent::Data { data }) => {
                self.screen.feed(&data);
                if let Some(title) = self.screen.take_title() {
                    self.title = title;
                }
                for reply in self.screen.take_replies(
                    f32::from(self.cell_width) as u16,
                    self.preferences.line_pixels() as u16,
                ) {
                    self.send_input(reply);
                }
            }
            Update::Pty(TerminalEvent::Exit { code, signal }) => {
                self.phase = Phase::Exited(match (code, signal) {
                    (_, Some(signal)) => format!("Terminal exited ({signal})"),
                    (Some(code), _) => format!("Terminal exited ({code})"),
                    _ => "Terminal exited".into(),
                });
                self.marked.clear();
            }
            Update::Pty(TerminalEvent::Error { message }) | Update::Error(message) => {
                self.input_error = Some(message);
            }
        }
    }

    fn send_input(&mut self, input: String) {
        if !matches!(self.phase, Phase::Running | Phase::Starting) || input.is_empty() {
            return;
        }
        if let Some(worker) = &self.worker {
            match worker.send(Command::Input(input)) {
                Ok(()) => self.input_error = None,
                Err(error) => self.input_error = Some(error),
            }
        }
    }

    fn user_input(&mut self, input: String) {
        self.screen.clear_selection();
        self.screen.scroll_to_bottom();
        self.send_input(input);
    }

    fn copy(&self, cx: &mut App) {
        if let Some(text) = self.screen.selected_text().filter(|text| !text.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            if text.len() > MAX_PASTE {
                self.input_error = Some("Paste exceeds 1 MiB".into());
            } else {
                let text = text.replace("\r\n", "\n").replace('\r', "\n");
                let text = if self.screen.mode().contains(TermMode::BRACKETED_PASTE) {
                    // Prevent pasted escape bytes from terminating bracketed paste.
                    format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', ""))
                } else {
                    text.replace('\n', "\r")
                };
                self.user_input(text);
            }
            cx.notify();
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        let mods = key.modifiers;
        let shortcut = mods.platform || (mods.control && mods.shift);
        if shortcut && key.key == "c" {
            self.copy(cx);
        } else if shortcut && key.key == "v" {
            self.paste(cx);
        } else if !self.marked.is_empty() || event.prefer_character_input {
            // Let the platform IME consume candidate navigation and confirmation.
            return;
        } else if mods.shift && matches!(key.key.as_str(), "pageup" | "pagedown") {
            let lines = self.screen.dimensions().1 as i32;
            self.screen
                .scroll(if key.key == "pageup" { lines } else { -lines });
        } else if let Some(input) = encode_key(key, self.screen.mode()) {
            self.user_input(input);
        } else {
            // Printable text arrives through EntityInputHandler, not key_char.
            return;
        }
        window.notify_text_input_changed(&self.focus_handle);
        cx.stop_propagation();
        cx.notify();
    }

    fn hit(&self, position: Point<Pixels>) -> (usize, usize, bool) {
        let x = f32::from(position.x - self.bounds.origin.x) / f32::from(self.cell_width);
        let y = f32::from(position.y - self.bounds.origin.y) / self.preferences.line_pixels();
        let (columns, rows) = self.screen.dimensions();
        selection_point(x, y, columns, rows)
    }

    fn prepare(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PaintState {
        if self.geometry_dirty {
            let measure = shape(
                "M".into(),
                self.fonts[0].clone(),
                self.screen.resolved_palette().foreground,
                self.preferences.font_size,
                window,
            );
            self.cell_width = measure.width.max(px(1.));
            self.geometry_dirty = false;
        }
        self.bounds = bounds;
        let columns = (f32::from(bounds.size.width) / f32::from(self.cell_width))
            .floor()
            .max(2.) as usize;
        let rows = (f32::from(bounds.size.height) / self.preferences.line_pixels())
            .floor()
            .max(1.) as usize;
        if self.screen.dimensions() != (columns, rows) {
            self.screen.resize(columns, rows);
        }
        let dimensions = self.screen.dimensions();
        if self.last_resize != Some(dimensions)
            && matches!(self.phase, Phase::Running)
            && let Some(worker) = &self.worker
        {
            match worker.send(Command::Resize(dimensions.0, dimensions.1)) {
                Ok(()) => self.last_resize = Some(dimensions),
                Err(error) => self.input_error = Some(error),
            }
        }
        let snapshot = self.screen.snapshot();
        let focused = self.focus_handle.is_focused(window);
        self.focused = focused;
        self.last_paint = Instant::now();
        let glyphs = snapshot
            .cells
            .iter()
            .filter_map(|cell| {
                if cell.width == 0 || cell.text.trim().is_empty() {
                    return None;
                }
                let cursor = snapshot.cursor.is_some_and(|cursor| {
                    cursor.row == cell.row
                        && cursor.column == cell.column
                        && cursor.shape == CursorShape::Block
                        && focused
                        && (!cursor.blinking || self.blink_visible)
                }) && self.marked.is_empty();
                let foreground = if cursor {
                    snapshot.background
                } else {
                    cell.foreground
                };
                Some((
                    cell.row,
                    cell.column,
                    shape(
                        cell.text.clone().into(),
                        self.fonts[usize::from(cell.bold) + 2 * usize::from(cell.italic)].clone(),
                        foreground,
                        self.preferences.font_size,
                        window,
                    ),
                ))
            })
            .collect();
        let cursor = snapshot.cursor;
        let previous_origin = self.marked_origin;
        self.marked_origin = cursor
            .map(|cursor| self.cell_origin(cursor.row, cursor.column))
            .unwrap_or(bounds.origin);
        self.marked_layout = (!self.marked.is_empty()).then(|| {
            shape(
                self.marked.clone().into(),
                self.fonts[0].clone(),
                snapshot.foreground,
                self.preferences.font_size,
                window,
            )
        });
        // Candidate positioning follows the terminal cursor, not the whole surface.
        if previous_origin != self.marked_origin {
            window.invalidate_character_coordinates();
        }
        let _ = cx;
        PaintState {
            snapshot,
            glyphs,
            focused,
        }
    }

    fn cell_origin(&self, row: usize, column: usize) -> Point<Pixels> {
        self.bounds.origin
            + point(
                self.cell_width * column as f32,
                px(self.preferences.line_pixels() * row as f32),
            )
    }

    fn paint(&self, state: PaintState, window: &mut Window, cx: &mut App) {
        window.paint_quad(fill(self.bounds, rgb(state.snapshot.background)));
        for cell in &state.snapshot.cells {
            if cell.background != state.snapshot.background || cell.selected {
                let bounds = Bounds::new(
                    self.cell_origin(cell.row, cell.column),
                    size(self.cell_width, px(self.preferences.line_pixels())),
                );
                window.paint_quad(fill(
                    bounds,
                    if cell.selected {
                        rgba(self.screen.palette().selection.unwrap_or(0x38516e88))
                    } else {
                        rgb(cell.background)
                    },
                ));
            }
        }
        if let Some(cursor) = state.snapshot.cursor.filter(|cursor| {
            self.marked.is_empty() && (!state.focused || !cursor.blinking || self.blink_visible)
        }) {
            let bounds = Bounds::new(
                self.cell_origin(cursor.row, cursor.column),
                size(
                    self.cell_width * cursor.width as f32,
                    px(self.preferences.line_pixels()),
                ),
            );
            if !state.focused {
                window.paint_quad(outline(bounds, rgb(cursor.color), BorderStyle::Solid));
            } else {
                let bounds = match cursor.shape {
                    CursorShape::Underline => Bounds::new(
                        point(bounds.left(), bounds.bottom() - px(2.)),
                        size(bounds.size.width, px(2.)),
                    ),
                    CursorShape::Beam => {
                        Bounds::new(bounds.origin, size(px(2.), bounds.size.height))
                    }
                    _ => bounds,
                };
                window.paint_quad(fill(bounds, rgb(cursor.color)));
            }
        }
        for (row, column, line) in state.glyphs {
            if let Err(error) = line.paint(
                self.cell_origin(row, column),
                px(self.preferences.line_pixels()),
                TextAlign::Left,
                None,
                window,
                cx,
            ) {
                tracing::debug!("terminal glyph paint: {error}");
            }
        }
        for cell in &state.snapshot.cells {
            if cell.width == 0 {
                continue;
            }
            let origin = self.cell_origin(cell.row, cell.column);
            for y in [
                cell.underline
                    .then_some(self.preferences.line_pixels() - 2.),
                cell.strikethrough
                    .then_some(self.preferences.line_pixels() * 0.55),
            ]
            .into_iter()
            .flatten()
            {
                window.paint_quad(fill(
                    Bounds::new(
                        origin + point(px(0.), px(y)),
                        size(self.cell_width * cell.width as f32, px(1.)),
                    ),
                    rgb(cell.foreground),
                ));
            }
        }
        if let Some(line) = &self.marked_layout {
            let width = line.width.max(self.cell_width);
            window.paint_quad(fill(
                Bounds::new(
                    self.marked_origin,
                    size(width, px(self.preferences.line_pixels())),
                ),
                rgb(state.snapshot.background),
            ));
            let _ = line.paint(
                self.marked_origin,
                px(self.preferences.line_pixels()),
                TextAlign::Left,
                None,
                window,
                cx,
            );
            window.paint_quad(fill(
                Bounds::new(
                    self.marked_origin + point(px(0.), px(self.preferences.line_pixels() - 1.)),
                    size(width, px(1.)),
                ),
                rgb(state.snapshot.foreground),
            ));
            let offset = utf16_to_byte(&self.marked, self.marked_selection.end);
            let x = line.x_for_index(offset);
            window.paint_quad(fill(
                Bounds::new(
                    self.marked_origin + point(x, px(0.)),
                    size(px(1.), px(self.preferences.line_pixels())),
                ),
                rgb(state.snapshot.foreground),
            ));
        }
    }
}

impl Focusable for TerminalPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TerminalPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let preferences = cx
            .try_global::<TerminalPreferences>()
            .cloned()
            .unwrap_or_default();
        if self.preferences != preferences {
            if !self.preferences.geometry_eq(&preferences) {
                self.geometry_dirty = true;
                self.marked_layout = None;
            }
            if self.preferences.font_family != preferences.font_family {
                self.fonts = terminal_fonts(&preferences.font_family);
            }
            self.screen.apply_preferences(&preferences);
            self.preferences = preferences;
            self.blink_visible = true;
        }
        let palette = TerminalPalette::scheme(&self.preferences.scheme, cx.theme().is_dark());
        if self.screen.palette() != palette {
            self.screen.set_palette(palette);
        }
        let colors = self.screen.resolved_palette();
        let entity = cx.entity();
        let prepaint_entity = entity.clone();
        let paint_entity = entity.clone();
        let status = self.status();
        div()
            .id("terminal-panel")
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(colors.background))
            .text_color(rgb(colors.foreground))
            .track_focus(&self.focus_handle)
            .key_context("Terminal")
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.focus(window, cx);
                    let (row, column, right) = this.hit(event.position);
                    this.screen
                        .begin_selection(row, column, right, event.click_count);
                    this.selecting = true;
                    window.prevent_default();
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.selecting {
                    let (row, column, right) = this.hit(event.position);
                    this.screen.extend_selection(row, column, right);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.selecting = false;
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.selecting = false;
                }),
            )
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
                this.scroll_remainder += f32::from(
                    event
                        .delta
                        .pixel_delta(px(this.preferences.line_pixels()))
                        .y,
                ) / this.preferences.line_pixels();
                let lines = this.scroll_remainder.trunc() as i32;
                this.scroll_remainder -= lines as f32;
                if lines != 0 {
                    this.screen.scroll(lines);
                    cx.notify();
                }
                cx.stop_propagation();
            }))
            .children(status.map(|status| div().flex_none().px_2().py_1().text_sm().child(status)))
            .child(
                div().relative().flex_1().min_h_0().overflow_hidden().child(
                    canvas(
                        move |bounds, window, cx| {
                            prepaint_entity.update(cx, |this, cx| this.prepare(bounds, window, cx))
                        },
                        move |bounds, state, window, cx| {
                            let focus = paint_entity.read(cx).focus_handle.clone();
                            window.handle_input(
                                &focus,
                                ElementInputHandler::new(bounds, entity),
                                cx,
                            );
                            paint_entity.update(cx, |this, cx| this.paint(state, window, cx));
                        },
                    )
                    .size_full(),
                ),
            )
    }
}

struct PaintState {
    snapshot: ScreenSnapshot,
    glyphs: Vec<(usize, usize, ShapedLine)>,
    focused: bool,
}

/// Coordinates are measured in terminal cells, not window pixels.
fn selection_point(x: f32, y: f32, columns: usize, rows: usize) -> (usize, usize, bool) {
    let x = x.clamp(0., columns as f32);
    (
        y.max(0.).floor().min((rows - 1) as f32) as usize,
        x.floor().min((columns - 1) as f32) as usize,
        // Beyond the last column is always its right edge, regardless of
        // the fractional position in a nonexistent cell outside the grid.
        x >= columns as f32 || x.fract() >= 0.5,
    )
}

fn terminal_fonts(family: &str) -> [Font; 4] {
    std::array::from_fn(|index| terminal_font(family, index & 1 != 0, index & 2 != 0))
}

fn terminal_font(family: &str, bold: bool, italic: bool) -> Font {
    let mut font = font(if family.trim().is_empty() {
        "JetBrainsMono Nerd Font Mono".to_owned()
    } else {
        family.trim().to_owned()
    });
    font.features = FontFeatures::disable_ligatures();
    if bold {
        font.weight = FontWeight::BOLD;
    }
    if italic {
        font.style = FontStyle::Italic;
    }
    font
}

fn shape(
    text: SharedString,
    font: Font,
    color: u32,
    font_size: f32,
    window: &Window,
) -> ShapedLine {
    let len = text.len();
    window.text_system().shape_line(
        text,
        px(font_size),
        &[TextRun {
            len,
            font,
            color: rgb(color).into(),
            ..Default::default()
        }],
        None,
    )
}

fn encode_key(key: &Keystroke, mode: TermMode) -> Option<String> {
    let modifiers = key.modifiers;
    if modifiers.platform {
        return None;
    }
    if modifiers.control && !modifiers.alt {
        let byte = match key.key.as_str() {
            "space" | "@" | "2" => Some(0),
            "[" | "3" => Some(27),
            "\\" | "4" => Some(28),
            "]" | "5" => Some(29),
            "^" | "6" => Some(30),
            "_" | "-" | "7" => Some(31),
            "?" | "8" => Some(127),
            value if value.len() == 1 && value.as_bytes()[0].is_ascii_alphabetic() => {
                Some(value.as_bytes()[0].to_ascii_uppercase() & 0x1f)
            }
            _ => None,
        };
        if let Some(byte) = byte {
            return Some((byte as char).to_string());
        }
    }
    let modifier = 1
        + usize::from(modifiers.shift)
        + 2 * usize::from(modifiers.alt)
        + 4 * usize::from(modifiers.control);
    let input = match key.key.as_str() {
        "enter" => if mode.contains(TermMode::LINE_FEED_NEW_LINE) {
            "\r\n"
        } else {
            "\r"
        }
        .into(),
        "backspace" => if modifiers.alt { "\x1b\x7f" } else { "\x7f" }.into(),
        "escape" => "\x1b".into(),
        "tab" => if modifiers.shift { "\x1b[Z" } else { "\t" }.into(),
        "up" | "down" | "right" | "left" | "home" | "end" => {
            let suffix = match key.key.as_str() {
                "up" => 'A',
                "down" => 'B',
                "right" => 'C',
                "left" => 'D',
                "home" => 'H',
                _ => 'F',
            };
            if modifier != 1 {
                format!("\x1b[1;{modifier}{suffix}")
            } else if mode.contains(TermMode::APP_CURSOR) {
                format!("\x1bO{suffix}")
            } else {
                format!("\x1b[{suffix}")
            }
        }
        "insert" | "delete" | "pageup" | "pagedown" | "f5" | "f6" | "f7" | "f8" | "f9" | "f10"
        | "f11" | "f12" => {
            let code = match key.key.as_str() {
                "insert" => 2,
                "delete" => 3,
                "pageup" => 5,
                "pagedown" => 6,
                "f5" => 15,
                "f6" => 17,
                "f7" => 18,
                "f8" => 19,
                "f9" => 20,
                "f10" => 21,
                "f11" => 23,
                _ => 24,
            };
            if modifier == 1 {
                format!("\x1b[{code}~")
            } else {
                format!("\x1b[{code};{modifier}~")
            }
        }
        "f1" | "f2" | "f3" | "f4" => {
            let suffix = match key.key.as_str() {
                "f1" => 'P',
                "f2" => 'Q',
                "f3" => 'R',
                _ => 'S',
            };
            if modifier == 1 {
                format!("\x1bO{suffix}")
            } else {
                format!("\x1b[1;{modifier}{suffix}")
            }
        }
        _ => return None,
    };
    Some(input)
}

fn utf16_to_byte(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (byte, character) in text.char_indices() {
        if units + character.len_utf16() > offset {
            return byte;
        }
        units += character.len_utf16();
    }
    text.len()
}

impl EntityInputHandler for TerminalPanel {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let start = utf16_to_byte(&self.marked, range.start);
        let end = utf16_to_byte(&self.marked, range.end).max(start);
        *actual = Some(
            self.marked[..start].encode_utf16().count()..self.marked[..end].encode_utf16().count(),
        );
        Some(self.marked[start..end].into())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.marked_selection.clone(),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.marked.is_empty()).then(|| 0..self.marked.encode_utf16().count())
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.marked.clear();
        self.marked_selection = 0..0;
        self.marked_layout = None;
        window.notify_text_input_changed(&self.focus_handle);
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked.clear();
        self.marked_selection = 0..0;
        self.marked_layout = None;
        self.user_input(text.into());
        window.notify_text_input_changed(&self.focus_handle);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range.unwrap_or(0..self.marked.encode_utf16().count());
        let start = utf16_to_byte(&self.marked, range.start);
        let end = utf16_to_byte(&self.marked, range.end).max(start);
        let insertion = self.marked[..start].encode_utf16().count();
        self.marked.replace_range(start..end, text);
        let len = text.encode_utf16().count();
        self.marked_selection = selected
            .map(|range| insertion + range.start.min(len)..insertion + range.end.min(len))
            .unwrap_or(insertion + len..insertion + len);
        self.marked_layout = None;
        window.notify_text_input_changed(&self.focus_handle);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (start, end) = if let Some(line) = &self.marked_layout {
            (
                line.x_for_index(utf16_to_byte(&self.marked, range.start)),
                line.x_for_index(utf16_to_byte(&self.marked, range.end)),
            )
        } else {
            (px(0.), self.cell_width)
        };
        Some(Bounds::new(
            self.marked_origin + point(start, px(0.)),
            size(
                (end - start).max(px(1.)),
                px(self.preferences.line_pixels()),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let line = self.marked_layout.as_ref()?;
        let byte = line.closest_index_for_x(position.x - self.marked_origin.x);
        Some(self.marked[..byte].encode_utf16().count())
    }
}

#[cfg(test)]
mod tests {
    use super::selection_point;
    use crate::features::terminal::screen::TerminalScreen;

    #[test]
    fn dragging_past_the_right_edge_keeps_the_last_cell_selected() {
        let mut terminal = TerminalScreen::new(3, 1);
        terminal.feed(b"abc");
        terminal.begin_selection(0, 0, false, 1);
        for x in [3., 3.1, 4.1, 10.1] {
            let (row, column, right) = selection_point(x, 0., 3, 1);
            assert_eq!((row, column, right), (0, 2, true));
            terminal.extend_selection(row, column, right);
            assert_eq!(terminal.selected_text().as_deref(), Some("abc"));
        }
    }

    #[test]
    fn selection_hit_preserves_half_cells_and_clamps_viewport_edges() {
        assert_eq!(selection_point(1.49, 1.9, 3, 2), (1, 1, false));
        assert_eq!(selection_point(1.5, 1.9, 3, 2), (1, 1, true));
        assert_eq!(selection_point(-0.1, -0.1, 3, 2), (0, 0, false));
        assert_eq!(selection_point(2.9, 100., 3, 2), (1, 2, true));
    }
}
