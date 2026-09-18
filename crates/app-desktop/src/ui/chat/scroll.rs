//! `ScrollController` — the single owner of the chat stream's scroll logic.
//!
//! Follows the bottom of the stream until the user scrolls up (detached),
//! re-arms when they scroll back down. One entry point for every scroll
//! decision so `update.rs` never re-implements the follow/detach dance
//! inline. iced's `scrollable` does the actual rendering; this controller
//! only decides WHEN to snap and whether the user has taken over.

use iced::widget::{operation, scrollable, Id};
use iced::Task;

use crate::ui::message::Message;

/// Near-bottom threshold — `relative_offset().y` above this still counts as
/// "at the bottom" so a hair of slack doesn't detach the follow.
const FOLLOW_EPSILON: f32 = 0.98;

/// Owns the stream's scroll identity + the user's detach state.
pub struct ScrollController {
    /// The widget `Id` `snap_to_end` targets.
    id: Id,
    /// True once the user scrolls away from the bottom — the stream stops
    /// auto-following until they scroll back down or switch sessions.
    detached: bool,
}

impl Default for ScrollController {
    fn default() -> Self {
        Self { id: Id::unique(), detached: false }
    }
}

impl ScrollController {
    /// The widget `Id` the stream scrollable registers itself under.
    pub fn id(&self) -> Id {
        self.id.clone()
    }

    /// `scrollable::on_scroll` feed — detach when the user leaves the
    /// bottom, re-arm when they return.
    pub fn on_scroll(&mut self, viewport: scrollable::Viewport) {
        self.detached = viewport.relative_offset().y < FOLLOW_EPSILON;
    }

    /// Should the stream chase new content? Only while the user hasn't
    /// detached. Part of the controller's public surface — callers that
    /// need the flag itself (rather than a snap task) read it here.
    #[allow(dead_code)]
    pub fn should_follow(&self) -> bool {
        !self.detached
    }

    /// A `snap_to_end` task — hard jump to the bottom (no animation, no
    /// jitter). Fires once per frame after the delta flush.
    pub fn snap_to_bottom(&self) -> Task<Message> {
        operation::snap_to_end(self.id.clone())
    }

    /// Snap only if the user hasn't detached — the common "new content
    /// landed" path.
    pub fn snap_if_following(&self) -> Task<Message> {
        if self.detached {
            Task::none()
        } else {
            self.snap_to_bottom()
        }
    }

    /// Reset the detach flag — session switch / fresh session re-arms the
    /// follow so the stream opens glued to the latest.
    pub fn reset(&mut self) {
        self.detached = false;
    }
}
