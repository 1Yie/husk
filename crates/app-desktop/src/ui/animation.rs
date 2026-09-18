//! Animation — the app's single motion vocabulary, built on `iced_anim`.
//!
//! Two families:
//! - `AnimationBuilder` helpers for **target-driven** transitions (expand,
//!   fade-in, pop) — the widget owns its `Animated` value and self-ticks on
//!   `RedrawRequested`, so no message plumbing is needed.
//! - `Animated<f32>` state helpers for **looping** pulses (loading dots,
//!   streaming caret) — stored on the view state and advanced by the shared
//!   `Message::Tick` pump in `update`.
//!
//! All curves come from `iced_anim::spring::Motion` / `transition::Easing`;
//! nothing here interpolates by hand.

use std::time::Duration;

use iced::{Color, Element};
use iced_anim::spring::Motion;
use iced_anim::transition::Easing;
use iced_anim::Animated;

// Re-exported so callers write `animation::AnimationBuilder` — keeps the
// whole motion vocabulary behind this one module.
pub use iced_anim::animation_builder::AnimationBuilder;

use super::message::Message;

/// Snappy spring for layout changes (expand/collapse) — quick settle, no
/// overshoot so a diff panel doesn't bounce.
pub const SPRING_LAYOUT: Motion = Motion {
    damping: 0.7,
    response: Duration::from_millis(220),
};

/// Bouncy spring for "a thing appeared" pops (awaiting-confirm capsule).
/// Reserved for the next pop surface — kept in the vocabulary so callers
/// reach for a shared curve instead of hand-tuning one.
#[allow(dead_code)]
pub const SPRING_POP: Motion = Motion {
    damping: 0.5,
    response: Duration::from_millis(320),
};

/// Gentle ease for fades — content arriving/settling.
#[allow(dead_code)]
pub const EASE_FADE: Easing = Easing::EASE_OUT;

/// Braille spinner frames — a real "转圈圈" indicator that cycles through
/// rotating glyphs each Tick. Used anywhere a turn is in-flight (sidebar
/// running marker, composer thinking indicator).
pub const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// The spinner glyph for a tick counter — `tick/2` steps at ~30fps on the
/// 60Hz tick, fast enough to read as rotation.
pub fn spinner_at(tick: u64) -> char {
    SPINNER[((tick / 2) as usize) % SPINNER.len()]
}

/// Animate a container's expansion between 0 and `full` — used for the
/// tool-call expanded body (diff/output) and the reasoning block. `open`
/// selects the target; the builder springs the height multiplier.
///
/// `content` receives a `f32` in `0..=1` so callers can also fade/slide
/// during the move.
pub fn expand<'a>(
    open: bool,
    content: impl Fn(f32) -> Element<'a, Message> + 'a,
) -> AnimationBuilder<'a, f32, Message, iced::Theme, iced::Renderer> {
    AnimationBuilder::new(if open { 1.0 } else { 0.0 }, move |t| content(t))
        .animation(SPRING_LAYOUT)
        .animates_layout(true)
}

/// Pop an element in with a bouncy spring — scale/opacity 0→1. Used when a
/// fresh interactive surface appears (the awaiting-confirm capsule).
#[allow(dead_code)]
pub fn pop<'a>(
    appear: bool,
    content: impl Fn(f32) -> Element<'a, Message> + 'a,
) -> AnimationBuilder<'a, f32, Message, iced::Theme, iced::Renderer> {
    AnimationBuilder::new(if appear { 1.0 } else { 0.0 }, move |t| content(t))
        .animation(SPRING_POP)
        .animates_layout(true)
}

/// A looping `Animated<f32>` phase for pulse animations (loading dots,
/// streaming caret). Stored on view state; `Message::Tick` calls
/// [`pulse_tick`] to advance it. The phase bounces 0→1→0 forever — read
/// `.value()` and map it onto opacity/scale.
pub type Pulse = Animated<f32>;

/// Start a looping pulse at `period` per half-cycle (0→1 leg). Uses a
/// smooth ease so the pulse breathes rather than clicks.
pub fn pulse(period_ms: u64) -> Pulse {
    Animated::transition(0.0, Easing::EASE_IN_OUT.with_duration(Duration::from_millis(period_ms)))
        .to(1.0)
}

/// Advance a looping pulse one frame — call from `Message::Tick`. When the
/// phase reaches an end it flips the target so it breathes 0→1→0→1 forever.
pub fn pulse_tick(pulse: &mut Pulse, now: std::time::Instant) {
    pulse.tick(now);
    if !pulse.is_animating() {
        // Settled — flip the target to keep the loop alive.
        let next = if *pulse.target() > 0.5 { 0.0 } else { 1.0 };
        pulse.set_target(next);
    }
}

/// Interpolate a color's alpha channel by `t` (0→1) — helper for fades.
/// Keeps rgb, scales a.
pub fn fade(color: Color, t: f32) -> Color {
    let mut c = color;
    c.a *= t.clamp(0.0, 1.0);
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn pulse_loops() {
        let mut p = pulse(100);
        let now = Instant::now();
        // Fresh pulse targets 1.0 and is animating.
        assert_eq!(*p.target(), 1.0);
        // After many ticks past the 100ms duration it should settle, then
        // `pulse_tick` flips the target back to 0 so the loop continues.
        let later = now + Duration::from_millis(500);
        pulse_tick(&mut p, later);
        assert!(*p.target() <= 0.5, "pulse flips target after settling");
    }

    #[test]
    fn fade_scales_alpha() {
        let c = Color::from_rgba(1.0, 0.5, 0.0, 1.0);
        let half = fade(c, 0.5);
        assert!((half.a - 0.5).abs() < 1e-3);
        let clamped = fade(c, 5.0);
        assert!((clamped.a - 1.0).abs() < 1e-3);
    }
}
