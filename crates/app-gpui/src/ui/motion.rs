use std::time::Duration;

use gpui_kit::base::motion::{self, Easing, Transition, TransitionId};
use gpui_kit::*;

// Keep these with their web counterparts: dock/index.tsx, ui/dialog.tsx,
// ui/dropdown-menu.tsx and tailwind.config.js's collapsible keyframes.
pub const PANEL_DURATION: Duration = Duration::from_millis(300);
pub const DIALOG_DURATION: Duration = Duration::from_millis(200);
pub const MENU_DURATION: Duration = Duration::from_millis(100);
pub const POPOVER_DURATION: Duration = Duration::from_millis(150);
pub const SCROLL_DURATION: Duration = Duration::from_millis(180);

pub fn panel_transition() -> Transition {
    Transition::new(PANEL_DURATION).easing(Easing::EaseOut)
}

pub fn reveal_transition(open: bool) -> Transition {
    Transition::new(Duration::from_millis(if open { 250 } else { 200 })).easing(
        Easing::CubicBezier {
            x1: 0.16,
            y1: 1.,
            x2: 0.3,
            y2: 1.,
        },
    )
}

pub fn panel_progress(
    id: impl Into<TransitionId>,
    visible: bool,
    window: &mut Window,
    cx: &mut App,
) -> f32 {
    motion::transition(
        id,
        if visible { 1. } else { 0. },
        panel_transition(),
        window,
        cx,
    )
}

/// A wheel's discrete line steps ease toward their destination; a trackpad's
/// pixel deltas and direct scrollbar manipulation remain native and immediate.
/// Sampling only while a destination exists avoids a perpetual frame loop.
#[derive(Default)]
pub struct SmoothScroll {
    target: Option<Point<Pixels>>,
    applied: Option<Point<Pixels>>,
    generation: u64,
}

impl SmoothScroll {
    pub fn on_wheel(&mut self, event: &ScrollEvent, handle: &ScrollHandle) -> bool {
        let ScrollDelta::Lines(lines) = event.delta else {
            self.cancel();
            return false;
        };
        if lines.y == 0. || lines.x != 0. {
            self.cancel();
            return false;
        }
        let max = handle.max_offset();
        if max.y <= px(0.) {
            return false;
        }
        let current = handle.offset();
        let from = self
            .target
            .filter(|_| self.applied == Some(current))
            .unwrap_or(current);
        let target = point(
            current.x,
            px(wheel_target(from.y.into(), lines.y, max.y.into())),
        );
        if target.y == from.y {
            return false;
        }
        self.target = Some(target);
        self.applied = Some(current);
        true
    }

    pub fn sample(
        &mut self,
        id: &'static str,
        handle: &ScrollHandle,
        window: &mut Window,
        cx: &mut App,
    ) {
        let current = handle.offset();
        // A drag, native trackpad scroll or an explicit scroll-to wins over an
        // earlier wheel animation instead of being pulled back on the next frame.
        if self.applied.is_some_and(|applied| applied != current) {
            self.cancel();
        }
        let target = self.target.unwrap_or(current);
        let sampled = motion::transition_with_status(
            (
                ElementId::Name(format!("{id}:{}", self.generation).into()),
                "offset",
            ),
            target,
            Transition::new(SCROLL_DURATION).easing(Easing::EaseOut),
            window,
            cx,
        );
        if self.target.is_some() {
            let max = handle.max_offset();
            let offset = point(sampled.value.x, sampled.value.y.clamp(-max.y, px(0.)));
            handle.set_offset(offset);
            self.applied = Some(offset);
            if sampled.status == motion::MotionStatus::Finished {
                self.target = None;
            }
        }
    }

    fn cancel(&mut self) {
        self.target = None;
        self.applied = None;
        self.generation = self.generation.wrapping_add(1);
    }
}

fn wheel_target(current: f32, lines: f32, maximum: f32) -> f32 {
    (current + lines * 32.).clamp(-maximum.max(0.), 0.)
}

#[cfg(test)]
mod tests {
    use super::wheel_target;

    #[test]
    fn wheel_targets_accumulate_and_clamp_to_content() {
        assert_eq!(wheel_target(-60., -3., 400.), -156.);
        assert_eq!(wheel_target(-156., -3., 400.), -252.);
        assert_eq!(wheel_target(-390., -3., 400.), -400.);
        assert_eq!(wheel_target(-30., 3., 400.), 0.);
        assert_eq!(wheel_target(0., -3., 0.), 0.);
    }
}

#[cfg(all(test, feature = "test-support"))]
mod native_tests {
    use super::{PANEL_DURATION, panel_progress};
    use gpui_kit::{Context, IntoElement, Render, Styled, TestAppContext, Window, div, px};
    use std::{cell::Cell, rc::Rc, time::Duration};

    struct Panel {
        open: bool,
        progress: Rc<Cell<f32>>,
    }

    impl Render for Panel {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let progress = panel_progress("test-panel", self.open, window, cx);
            self.progress.set(progress);
            div().w(px(240. * progress)).h_full()
        }
    }

    #[gpui_kit::test]
    fn panel_closes_continuously_and_reverses_without_jumping(cx: &mut TestAppContext) {
        let progress = Rc::new(Cell::new(1.));
        let (view, cx) = cx.add_window_view({
            let progress = progress.clone();
            move |_, _| Panel {
                open: true,
                progress,
            }
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        view.update(cx, |panel, cx| {
            panel.open = false;
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(progress.get(), 1.);
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let closing = progress.get();
        assert!(closing > 0. && closing < 1.);
        view.update(cx, |panel, cx| {
            panel.open = true;
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!((progress.get() - closing).abs() < 0.001);
        cx.executor().advance_clock(PANEL_DURATION);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(progress.get(), 1.);
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
        });
        cx.run_until_parked();
        let callbacks = cx.update(|window, cx| window.simulate_next_frame(cx));
        assert_eq!(callbacks, 0, "a settled panel must not keep repainting");
    }

    #[gpui_kit::test]
    fn reduced_motion_changes_panel_geometry_in_one_frame(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let progress = Rc::new(Cell::new(1.));
        let (view, cx) = cx.add_window_view({
            let progress = progress.clone();
            move |_, _| Panel {
                open: true,
                progress,
            }
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        view.update(cx, |panel, cx| {
            panel.open = false;
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(progress.get(), 0.);
        cx.update(|window, cx| assert_eq!(window.simulate_next_frame(cx), 0));
    }
}
