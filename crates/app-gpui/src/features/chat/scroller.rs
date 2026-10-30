//! The kit scroller's ListState semantics, with pixel motion for explicit jumps.
//! Streaming only remeasures rows; it never starts or retargets a scroll.
use std::{cell::RefCell, collections::HashSet, ops::Range, rc::Rc, time::Duration};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    base::motion::{Easing, MotionStatus, Transition, transition_with_status},
    component::{ActiveTheme, Sizable, button::Button, scroll::ScrollableElement},
    *,
};

#[derive(Clone, Copy)]
struct Jump {
    pixels: Pixels,
    item: Option<usize>,
    follow_tail: bool,
}

pub(super) struct TranscriptState {
    pub list: ListState,
    jump: Option<Jump>,
    /// A revealing row must be measured until its motion settles.
    pub revealing: Rc<RefCell<HashSet<usize>>>,
}

impl TranscriptState {
    pub fn new(count: usize, cx: &mut Context<Self>) -> Self {
        let list = ListState::new(count, ListAlignment::Top, px(400.));
        list.set_follow_mode(FollowMode::Tail);
        let weak = cx.weak_entity();
        list.set_scroll_handler(move |_, _, cx| {
            let weak = weak.clone();
            cx.defer(move |cx| {
                let _ = weak.update(cx, |this, cx| {
                    // Wheel / trackpad / scrollbar always wins over a jump.
                    this.jump = None;
                    cx.notify();
                });
            });
        });
        let revealing = Rc::new(RefCell::new(HashSet::new()));
        let rows = revealing.clone();
        list.set_item_measurement_readiness(move |index, _| !rows.borrow().contains(&index));
        Self {
            list,
            jump: None,
            revealing,
        }
    }

    pub fn item_count(&self) -> usize {
        self.list.item_count()
    }

    pub fn is_scrolled_up(&self) -> bool {
        self.list.max_offset_for_scrollbar().y > px(0.)
            && !self.list.is_following_tail()
            && !self.list.is_scrolled_to_end().unwrap_or(false)
    }

    pub fn reset(&mut self, count: usize, cx: &mut Context<Self>) {
        self.jump = None;
        self.revealing.borrow_mut().clear();
        self.list.reset(count);
        self.list.set_follow_mode(FollowMode::Tail);
        cx.notify();
    }

    pub fn splice(&mut self, range: Range<usize>, count: usize, cx: &mut Context<Self>) {
        self.list.splice(range, count);
        cx.notify();
    }

    pub fn append(&mut self, count: usize, cx: &mut Context<Self>) {
        let end = self.item_count();
        self.splice(end..end, count, cx);
    }

    pub fn remeasure_items(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        self.list.remeasure_items(range);
        cx.notify();
    }

    pub fn jump_to(&mut self, item: Option<usize>, cx: &mut Context<Self>) {
        if item.is_some_and(|index| index >= self.item_count()) {
            return;
        }
        let original = self.list.logical_scroll_top();
        self.list.pause_following_tail();
        if let Some(item_ix) = item {
            self.list.scroll_to(ListOffset {
                item_ix,
                offset_in_item: px(0.),
            });
        } else {
            self.list.scroll_to_end();
        }
        let pixels = if item.is_none() {
            self.list.max_offset_for_scrollbar().y
        } else {
            -self.list.scroll_px_offset_for_scrollbar().y
        };
        self.list.scroll_to(original);
        self.jump = Some(Jump {
            pixels,
            item,
            follow_tail: item.is_none(),
        });
        cx.notify();
    }

    pub fn smooth_wheel(&mut self, event: &ScrollEvent, cx: &mut Context<Self>) -> bool {
        let ScrollDelta::Lines(lines) = event.delta else {
            // Pixel deltas already contain the touchpad's continuous motion.
            // Do not add a second easing or continue an older explicit jump.
            self.jump = None;
            return false;
        };
        if lines.y == 0. || lines.x != 0. {
            return false;
        }
        let maximum = self.list.max_offset_for_scrollbar().y;
        if maximum <= px(0.) {
            return false;
        }
        let current = -self.list.scroll_px_offset_for_scrollbar().y;
        let from = self
            .jump
            .filter(|jump| jump.item.is_none() && !jump.follow_tail)
            .map_or(current, |jump| jump.pixels);
        let target = px(wheel_destination(from.into(), lines.y, maximum.into()));
        if target == from {
            return false;
        }
        self.list.pause_following_tail();
        self.jump = Some(Jump {
            pixels: target,
            item: None,
            follow_tail: false,
        });
        cx.notify();
        true
    }

    pub fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        self.jump_to(None, cx);
    }
}

pub(super) fn transcript(
    id: SharedString,
    state: Entity<TranscriptState>,
    mut renderer: impl FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let (list_state, jump, scrolled_up) = {
        let state = state.read(cx);
        (state.list.clone(), state.jump, state.is_scrolled_up())
    };
    let current = -list_state.scroll_px_offset_for_scrollbar().y;
    // Keep a stationary baseline when idle. In particular, a manual wheel
    // movement must not leave a lagging transition to resume on the next jump.
    let motion = transition_with_status(
        (ElementId::Name(id.clone()), "scroll"),
        jump.map_or(current, |jump| jump.pixels),
        Transition::new(if jump.is_some() {
            Duration::from_millis(300)
        } else {
            Duration::ZERO
        })
        .easing(Easing::EaseOut),
        window,
        cx,
    );
    if let Some(jump) = jump {
        list_state.scroll_by(motion.value - current);
        if matches!(motion.status, MotionStatus::Finished | MotionStatus::Idle) {
            state.update(cx, |state, _| {
                if let Some(item_ix) = jump.item {
                    state.list.scroll_to(ListOffset {
                        item_ix,
                        offset_in_item: px(0.),
                    });
                } else if jump.follow_tail || jump.pixels >= state.list.max_offset_for_scrollbar().y
                {
                    state.list.set_follow_mode(FollowMode::Tail);
                    state.list.scroll_to_end();
                }
                state.jump = None;
            });
        }
    }
    let list = list(list_state.clone(), move |index, window, cx| {
        renderer(index, window, cx)
    })
    .size_full()
    .py_2();
    let jump_state = state.clone();
    let theme = cx.theme().clone();
    div()
        .id(id)
        .relative()
        .size_full()
        .min_h_0()
        .overflow_hidden()
        .child(
            div()
                .id("transcript-viewport")
                .size_full()
                .role(Role::Log)
                .child(list)
                .vertical_scrollbar(&list_state),
        )
        .when(scrolled_up, |this| {
            this.child(
                div()
                    .absolute()
                    .bottom_3()
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(
                        Button::new("jump-to-latest")
                            .small()
                            .rounded_full()
                            .border_1()
                            .border_color(theme.border)
                            .label("回到最新消息")
                            .on_click(move |_, _, cx| {
                                jump_state.update(cx, |state, cx| state.jump_to(None, cx))
                            }),
                    ),
            )
        })
        .into_any_element()
}

fn wheel_destination(current: f32, lines: f32, maximum: f32) -> f32 {
    (current - lines * 32.).clamp(0., maximum.max(0.))
}

#[cfg(test)]
mod wheel_tests {
    use super::wheel_destination;

    #[test]
    fn line_wheel_accumulates_without_crossing_either_edge() {
        assert_eq!(wheel_destination(100., -3., 1000.), 196.);
        assert_eq!(wheel_destination(196., -3., 1000.), 292.);
        assert_eq!(wheel_destination(50., 3., 1000.), 0.);
        assert_eq!(wheel_destination(950., -3., 1000.), 1000.);
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::TranscriptState;
    use gpui_kit::{AppContext, TestAppContext};

    #[gpui_kit::test]
    fn explicit_jump_preserves_off_tail_and_reset_cancels_motion(cx: &mut TestAppContext) {
        let state = cx.new(|cx| TranscriptState::new(3, cx));
        state.update(cx, |state, cx| {
            state.jump_to(Some(0), cx);
            assert!(state.jump.is_some());
            assert!(!state.list.is_following_tail());
            state.append(1, cx);
            assert!(!state.list.is_following_tail());
            state.reset(2, cx);
            assert!(state.jump.is_none());
            assert!(state.list.is_following_tail());
        });
    }
}
