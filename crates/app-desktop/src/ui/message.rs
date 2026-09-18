//! The `Message` enum — the only way `App` state changes. UI intents +
//! wrapped kernel events + the animation tick.


use iced::widget::scrollable;

/// Custom-titlebar window control.
#[derive(Debug, Clone, Copy)]
pub enum WinAction {
    Drag,
    Minimize,
    ToggleMaximize,
    Close,
}

#[derive(Debug, Clone)]
pub enum Message {
    // ---- UI → kernel intents ----
    InputChanged(String),
    Submit,
    Steer,
    /// Abort the in-flight turn (Esc / ✕ button) — writes the session's
    /// cancel flag directly so it isn't queued behind `run_turn`.
    Cancel,
    Approve(usize),
    Deny(usize),
    SelectSession(i64),
    /// Async history rebuild for `SelectSession` finished — carries the
    /// freshly-built `SessionView` (boxed: it's a big struct) + the scroll
    /// anchor to snap after install.
    SessionLoaded(i64, Box<crate::ui::state::SessionView>),
    NewSession,
    ToggleReasoning(usize),
    ToggleStep(usize),
    /// Copy a message's full text to the clipboard.
    CopyMessage(usize),
    /// Toggle a message into selectable-text mode (read-only editor).
    ToggleSelect(usize),
    /// A `text_editor` action on a selectable message — `Edit` is filtered
    /// out so the editor is read-only but still drag-selectable.
    SelectAction(usize, iced::widget::text_editor::Action),
    /// The OS window became ready — carries its `Id` for titlebar actions.
    WindowReady(Option<iced::window::Id>),
    /// Titlebar button — drag on press / minimize / toggle-maximize / close.
    WindowAction(WinAction),

    // ---- workspace intents ----
    /// Open system directory picker dialog.
    OpenWorkspace,
    /// Folder selected from dialog.
    WorkspaceSelected(std::path::PathBuf),
    /// Switch directly to a workspace path (e.g. from recent list).
    SwitchWorkspace(std::path::PathBuf),
    /// No-operation (e.g. dialog cancelled).
    Noop,

    // ---- frame ----
    /// ~60Hz pump — drains the kernel `std::sync::mpsc` + animates dots.
    Tick,
    /// Track user scroll position so auto-pin releases when they scroll up.
    Scrolled(scrollable::Viewport),
    /// A Markdown link was clicked — opened externally (no-op for now; the
    /// Uri is available for an `open::that` hook if we want it).
    LinkClicked(iced::widget::markdown::Uri),
}
