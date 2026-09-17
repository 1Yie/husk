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

    // ---- frame ----
    /// ~60Hz pump — drains the kernel `std::sync::mpsc` + animates dots.
    Tick,
    /// Track user scroll position so auto-pin releases when they scroll up.
    Scrolled(scrollable::Viewport),
    /// A Markdown link was clicked — opened externally (no-op for now; the
    /// Uri is available for an `open::that` hook if we want it).
    LinkClicked(iced::widget::markdown::Uri),
}
