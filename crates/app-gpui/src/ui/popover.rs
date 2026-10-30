use std::rc::Rc;
use std::time::Duration;

use gpui_kit::base::{
    PopoverState, Popup, StyledExt as _,
    actions::{Cancel, Confirm},
    motion::{Easing, Presence, Transition},
};
use gpui_kit::component::{ThemeStyled as _, menu::PopupMenu, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::motion::{MENU_DURATION, POPOVER_DURATION};

type Trigger = Box<dyn FnOnce(bool, &Window, &App) -> AnyElement>;
type Content =
    Box<dyn FnOnce(&mut PopoverState, &mut Window, &mut Context<PopoverState>) -> AnyElement>;

/// Non-modal native popup with the same focus lifecycle as GPUI's Popover,
/// retaining its surface through the exit rather than unmounting it on close.
#[derive(IntoElement)]
pub struct Popover {
    id: ElementId,
    style: StyleRefinement,
    anchor: Anchor,
    offset: Option<Pixels>,
    trigger: Option<Trigger>,
    content: Option<Content>,
    appearance: bool,
    disabled: bool,
    duration: Duration,
    mouse_button: MouseButton,
    on_open_change: Option<Rc<dyn Fn(&bool, &mut Window, &mut App)>>,
}

impl Popover {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            style: StyleRefinement::default(),
            anchor: Anchor::TopLeft,
            offset: None,
            trigger: None,
            content: None,
            appearance: true,
            disabled: false,
            duration: POPOVER_DURATION,
            mouse_button: MouseButton::Left,
            on_open_change: None,
        }
    }

    pub fn trigger(mut self, trigger: impl IntoElement + 'static) -> Self {
        self.trigger = Some(Box::new(move |_, _, _| trigger.into_any_element()));
        self
    }

    pub fn trigger_with(
        mut self,
        trigger: impl FnOnce(bool, &Window, &App) -> AnyElement + 'static,
    ) -> Self {
        self.trigger = Some(Box::new(trigger));
        self
    }

    pub fn content<E: IntoElement + 'static>(
        mut self,
        content: impl FnOnce(&mut PopoverState, &mut Window, &mut Context<PopoverState>) -> E + 'static,
    ) -> Self {
        self.content = Some(Box::new(move |state, window, cx| {
            content(state, window, cx).into_any_element()
        }));
        self
    }

    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn offset(mut self, offset: Pixels) -> Self {
        self.offset = Some(offset);
        self
    }

    pub fn appearance(mut self, appearance: bool) -> Self {
        self.appearance = appearance;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    pub fn mouse_button(mut self, button: MouseButton) -> Self {
        self.mouse_button = button;
        self
    }

    pub fn on_open_change(
        mut self,
        callback: impl Fn(&bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_change = Some(Rc::new(callback));
        self
    }
}

impl Styled for Popover {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Popover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state_id = ElementId::NamedChild(self.id.clone().into(), "state".into());
        let state = window.use_keyed_state(state_id, cx, |_, cx| PopoverState::new(false, cx));
        state.update(cx, |state, _| {
            state.set_on_open_change(self.on_open_change);
        });
        if self.disabled && state.read(cx).is_open() {
            state.update(cx, |state, cx| state.dismiss(window, cx));
        }
        let disabled = self.disabled;
        let open = state.read(cx).is_open();
        let focus = state.focus_handle(cx);
        let presence = Presence::new((self.id.clone(), "presence"), open)
            .transition(Transition::new(self.duration).easing(Easing::EaseOut))
            .sample(window, cx);
        let parent = window.current_view();
        let trigger = self
            .trigger
            .map(|trigger| trigger(open, window, cx))
            .unwrap_or_else(|| div().into_any_element());
        let popup = Popup::new(self.id, trigger)
            .anchor(self.anchor)
            .when_some(self.offset, |this, offset| this.offset(offset))
            .key_context("Popover")
            .on_action({
                let state = state.clone();
                move |_: &Confirm, window, cx| {
                    if disabled {
                        return;
                    }
                    state.update(cx, |state, cx| state.toggle_open(window, cx));
                    cx.notify(parent);
                }
            })
            .on_mouse_down(self.mouse_button, {
                let state = state.clone();
                move |_, window, cx| {
                    if disabled {
                        return;
                    }
                    cx.stop_propagation();
                    state.update(cx, |state, cx| {
                        if state.is_open() == open {
                            state.toggle_open(window, cx);
                        }
                    });
                    cx.notify(parent);
                }
            });
        if !presence.should_render() || (!open && presence.progress <= 0.) {
            return popup.into_any_element();
        }
        let progress = presence.progress.clamp(0., 1.);
        let direction = if matches!(self.anchor, Anchor::BottomLeft | Anchor::BottomRight) {
            1.
        } else {
            -1.
        };
        let content = v_flex()
            .id("animated-popover-content")
            .role(Role::Dialog)
            .tab_group()
            .when(self.appearance, |this| {
                this.popover_style(cx).rounded(px(18.)).p_3()
            })
            .refine_style(&self.style)
            // GPUI does not composite an entire subtree. Suppress the shadow
            // while translucent so it cannot show through the panel as a slab.
            .when(progress < 0.98, |this| this.shadow_none())
            .relative()
            .top(px(direction * 8. * (1. - progress)))
            .opacity(progress)
            .when(open, |this| {
                this.occlude()
                    .track_focus(&focus)
                    .key_context("PopoverContent")
                    .on_action({
                        let state = state.clone();
                        move |_: &Cancel, window, cx| {
                            state.update(cx, |state, cx| state.dismiss(window, cx));
                            cx.notify(parent);
                        }
                    })
                    .on_mouse_down_out({
                        let state = state.clone();
                        move |_, window, cx| {
                            state.update(cx, |state, cx| state.dismiss(window, cx));
                            cx.notify(parent);
                        }
                    })
            })
            .when_some(self.content, |this, content| {
                this.child(state.update(cx, |state, cx| content(state, window, cx)))
            })
            .when(!open, |this| {
                this.child(div().absolute().inset_0().occlude())
            });
        popup.content(content).into_any_element()
    }
}

type MenuBuilder = Box<dyn FnOnce(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

#[derive(Default)]
struct MenuCache {
    menu: Option<Entity<PopupMenu>>,
    was_open: bool,
    subscription: Option<Subscription>,
}

/// The native menu keeps arrow-key selection and submenus. Its entity is
/// retained between animation frames, but rebuilt for each opening so labels,
/// disabled states and callbacks never refer to an old session snapshot.
#[derive(IntoElement)]
pub struct Menu {
    id: ElementId,
    trigger: AnyElement,
    builder: MenuBuilder,
    anchor: Anchor,
    mouse_button: MouseButton,
    disabled: bool,
}

impl Menu {
    pub fn new(
        id: impl Into<ElementId>,
        trigger: impl IntoElement,
        builder: impl FnOnce(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            trigger: trigger.into_any_element(),
            builder: Box::new(builder),
            anchor: Anchor::TopRight,
            mouse_button: MouseButton::Left,
            disabled: false,
        }
    }

    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn mouse_button(mut self, button: MouseButton) -> Self {
        self.mouse_button = button;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for Menu {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let cache_id = ElementId::NamedChild(self.id.clone().into(), "menu".into());
        Popover::new(self.id)
            .disabled(self.disabled)
            .offset(px(4.))
            .duration(MENU_DURATION)
            .anchor(self.anchor)
            .mouse_button(self.mouse_button)
            .appearance(false)
            .trigger(self.trigger)
            .content(move |state, window, cx| {
                let open = state.is_open();
                let cache = window.use_keyed_state(cache_id, cx, |_, _| MenuCache::default());
                let recreate = {
                    let cache = cache.read(cx);
                    cache.menu.is_none() || (open && !cache.was_open)
                };
                if recreate {
                    let menu = PopupMenu::build(window, cx, self.builder);
                    let subscription =
                        cx.subscribe_in(&menu, window, |state, _, _: &DismissEvent, window, cx| {
                            state.dismiss(window, cx);
                            window.refresh();
                        });
                    if open {
                        menu.focus_handle(cx).focus(window, cx);
                    }
                    cache.update(cx, |cache, _| {
                        cache.menu = Some(menu);
                        cache.subscription = Some(subscription);
                    });
                }
                cache.update(cx, |cache, _| cache.was_open = open);
                cache.read(cx).menu.clone().unwrap()
            })
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::{MENU_DURATION, Menu, POPOVER_DURATION, Popover};
    use gpui_kit::component::button::Button;
    use gpui_kit::component::menu::PopupMenuItem;
    use gpui_kit::{
        Context, FocusHandle, InteractiveElement, IntoElement, ParentElement, Render, Styled,
        TestAppContext, Window, div, point, px,
    };
    use std::{cell::Cell, rc::Rc};

    struct Harness {
        focus: FocusHandle,
        builds: Rc<Cell<usize>>,
        disabled: bool,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let builds = self.builds.clone();
            div().size_full().child(
                Popover::new("test-popup")
                    .disabled(self.disabled)
                    .trigger(
                        Button::new("trigger")
                            .label("Open")
                            .track_focus(&self.focus),
                    )
                    .content(move |_, _, _| {
                        builds.set(builds.get() + 1);
                        div()
                            .debug_selector(|| "popup-surface".into())
                            .w(px(160.))
                            .h(px(80.))
                            .child("Popup")
                    }),
            )
        }
    }

    #[gpui_kit::test]
    fn escape_restores_focus_but_retains_the_surface_until_exit_finishes(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(|_, cx| Harness {
            focus: cx.focus_handle(),
            builds: Rc::new(Cell::new(0)),
            disabled: false,
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            let focus = view.read(cx).focus.clone();
            focus.focus(window, cx);
        });
        cx.simulate_keystrokes("enter");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.executor().advance_clock(POPOVER_DURATION);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("popup-surface").is_some());

        cx.simulate_keystrokes("escape");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(
            cx.debug_bounds("popup-surface").is_some(),
            "close must not unmount the first exit frame"
        );
        cx.update(|window, cx| assert!(view.read(cx).focus.is_focused(window)));
        cx.executor().advance_clock(POPOVER_DURATION);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("popup-surface").is_none());

        cx.simulate_keystrokes("enter");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.executor().advance_clock(POPOVER_DURATION);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(
            cx.debug_bounds("popup-surface").is_some(),
            "the same trigger can reopen after exit"
        );
    }

    #[gpui_kit::test]
    fn outside_click_dismisses_and_reduced_motion_does_not_retain_exit_frames(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        cx.update(|cx| cx.set_reduce_motion(true));
        let (_, cx) = cx.add_window_view(|_, cx| Harness {
            focus: cx.focus_handle(),
            builds: Rc::new(Cell::new(0)),
            disabled: false,
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_click(point(px(20.), px(12.)), Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("popup-surface").is_some());
        cx.simulate_click(point(px(500.), px(400.)), Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("popup-surface").is_none());
    }

    #[gpui_kit::test]
    fn disabled_popup_ignores_both_keyboard_and_pointer_activation(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(|_, cx| Harness {
            focus: cx.focus_handle(),
            builds: Rc::new(Cell::new(0)),
            disabled: true,
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            let focus = view.read(cx).focus.clone();
            focus.focus(window, cx);
        });
        cx.simulate_keystrokes("enter");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_click(point(px(20.), px(12.)), Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("popup-surface").is_none());
        view.read_with(cx, |view, _| assert_eq!(view.builds.get(), 0));
    }

    struct MenuHarness {
        focus: FocusHandle,
        pinned: Rc<Cell<bool>>,
        calls: Rc<Cell<usize>>,
    }

    impl Render for MenuHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let pinned = self.pinned.clone();
            let calls = self.calls.clone();
            let previous = pinned.get();
            Menu::new(
                "test-menu",
                Button::new("menu-trigger")
                    .label("Menu")
                    .track_focus(&self.focus),
                move |menu, _, _| {
                    menu.item(
                        PopupMenuItem::new(if previous { "取消固定" } else { "固定" }).on_click(
                            move |_, _, _| {
                                pinned.set(!previous);
                                calls.set(calls.get() + 1);
                            },
                        ),
                    )
                },
            )
        }
    }

    #[gpui_kit::test]
    fn menu_keeps_keyboard_selection_and_rebuilds_actions_each_open(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(|_, cx| MenuHarness {
            focus: cx.focus_handle(),
            pinned: Rc::new(Cell::new(false)),
            calls: Rc::new(Cell::new(0)),
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            let focus = view.read(cx).focus.clone();
            focus.focus(window, cx);
        });
        for expected in [true, false] {
            cx.simulate_keystrokes("enter");
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.executor().advance_clock(MENU_DURATION);
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.simulate_keystrokes("down enter");
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.executor().advance_clock(MENU_DURATION);
            cx.update(|window, cx| window.draw(cx).clear(cx));
            view.read_with(cx, |view, _| assert_eq!(view.pinned.get(), expected));
        }
        view.read_with(cx, |view, _| assert_eq!(view.calls.get(), 2));
    }
}
