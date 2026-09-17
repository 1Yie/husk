//! The `Message` enum — the only way `App` state changes. UI intents +
//! wrapped kernel events + the animation tick.

use iced::widget::scrollable;

#[derive(Debug, Clone)]
pub enum Message {
    // ---- UI → kernel intents ----
    InputChanged(String),
    Submit,
    Steer,
    Approve(usize),
    Deny(usize),
    SelectSession(i64),
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

    // ---- frame ----
    /// ~60Hz pump — drains the kernel `std::sync::mpsc` + animates dots.
    Tick,
    /// Track user scroll position so auto-pin releases when they scroll up.
    Scrolled(scrollable::Viewport),
    /// A Markdown link was clicked — opened externally (no-op for now; the
    /// Uri is available for an `open::that` hook if we want it).
    LinkClicked(iced::widget::markdown::Uri),
}
