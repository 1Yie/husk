//! iced UI module — `App` state, `Message`, `update`, `view`, `subscription`.
//! The kernel's `UiEvent` stream arrives via `std::sync::mpsc` and is polled
//! by a `Subscription` each tick — iced's Elm loop stays on the main thread.

pub mod animation;
pub mod chat;
pub mod icons;
pub mod layout;
pub mod message;
pub mod state;
pub mod theme;
pub mod update;
pub mod view;

use iced::{Subscription, time};
use std::time::Duration;

pub use message::Message;
pub use state::App;

impl App {
    /// Subscriptions: a ~60Hz tick that drains the kernel event queue AND
    /// advances the looping `iced_anim` pulses (loading dots, streaming
    /// carets). One batch per frame — no per-SSE-chunk updates.
    pub fn subscription(&self) -> Subscription<Message> {
        time::every(Duration::from_millis(16)).map(|_| Message::Tick)
    }
}
