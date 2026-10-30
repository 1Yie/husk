use std::path::PathBuf;

use gpui_kit::{
    component::{
        ActiveTheme, Disableable, Sizable,
        button::{Button, ButtonVariants},
        resizable::{ResizableState, h_resizable, resizable_panel, v_resizable},
    },
    prelude::*,
    *,
};
use husk_terminal::{MAX_TERMINALS_PER_WINDOW, TerminalState};

use super::{TerminalPanel, panel::TerminalPanelEvent};
use crate::ui::icons::Keyline;

const MAX_PANES: usize = MAX_TERMINALS_PER_WINDOW;

actions!(
    terminal,
    [
        NewTerminalTab,
        CloseTerminalPane,
        NextTerminalTab,
        PreviousTerminalTab,
        SplitTerminalColumns,
        SplitTerminalRows,
    ]
);

pub(crate) fn bind_keys(cx: &mut App) {
    let macos = cfg!(target_os = "macos");
    cx.bind_keys([
        KeyBinding::new(
            if macos { "cmd-t" } else { "ctrl-shift-t" },
            NewTerminalTab,
            Some("TerminalDeck"),
        ),
        KeyBinding::new(
            if macos { "cmd-w" } else { "ctrl-shift-w" },
            CloseTerminalPane,
            Some("TerminalDeck"),
        ),
        KeyBinding::new(
            if macos {
                "cmd-shift-]"
            } else {
                "ctrl-pagedown"
            },
            NextTerminalTab,
            Some("TerminalDeck"),
        ),
        KeyBinding::new(
            if macos { "cmd-shift-[" } else { "ctrl-pageup" },
            PreviousTerminalTab,
            Some("TerminalDeck"),
        ),
        KeyBinding::new(
            if macos { "cmd-d" } else { "ctrl-shift-d" },
            SplitTerminalColumns,
            Some("TerminalDeck"),
        ),
        KeyBinding::new(
            if macos { "cmd-shift-d" } else { "ctrl-shift-e" },
            SplitTerminalRows,
            Some("TerminalDeck"),
        ),
    ]);
}

fn cycled_index(current: Option<usize>, count: usize, forward: bool) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let Some(index) = current.filter(|index| *index < count) else {
        return Some(0);
    };
    Some(if forward {
        (index + 1) % count
    } else {
        (index + count - 1) % count
    })
}

pub enum TerminalDeckEvent {
    Hide,
}

#[derive(Clone, Copy)]
enum Axis {
    Columns,
    Rows,
}

struct Pane {
    id: u64,
    panel: Entity<TerminalPanel>,
    _focus: Subscription,
    _title: Subscription,
}

enum PaneTree {
    Leaf(Pane),
    Split {
        id: u64,
        state: Entity<ResizableState>,
        axis: Axis,
        first: Box<Self>,
        second: Box<Self>,
    },
}

impl PaneTree {
    fn count(&self) -> usize {
        match self {
            Self::Leaf(_) => 1,
            Self::Split { first, second, .. } => first.count() + second.count(),
        }
    }

    fn pane(&self, id: u64) -> Option<&Pane> {
        match self {
            Self::Leaf(pane) => (pane.id == id).then_some(pane),
            Self::Split { first, second, .. } => first.pane(id).or_else(|| second.pane(id)),
        }
    }

    fn first(&self) -> &Pane {
        match self {
            Self::Leaf(pane) => pane,
            Self::Split { first, .. } => first.first(),
        }
    }

    fn split(self, target: u64, axis: Axis, pane: Pane, state: Entity<ResizableState>) -> Self {
        match self {
            Self::Leaf(old) => Self::Split {
                id: pane.id,
                state,
                axis,
                first: Box::new(Self::Leaf(old)),
                second: Box::new(Self::Leaf(pane)),
            },
            Self::Split {
                id,
                state: old_state,
                axis: old_axis,
                first,
                second,
            } => {
                if first.pane(target).is_some() {
                    Self::Split {
                        id,
                        state: old_state,
                        axis: old_axis,
                        first: Box::new(first.split(target, axis, pane, state)),
                        second,
                    }
                } else {
                    Self::Split {
                        id,
                        state: old_state,
                        axis: old_axis,
                        first,
                        second: Box::new(second.split(target, axis, pane, state)),
                    }
                }
            }
        }
    }

    fn remove(self, target: u64) -> Option<Self> {
        match self {
            Self::Leaf(pane) => (pane.id != target).then_some(Self::Leaf(pane)),
            Self::Split {
                id,
                state,
                axis,
                first,
                second,
            } => match (first.remove(target), second.remove(target)) {
                (Some(first), Some(second)) => Some(Self::Split {
                    id,
                    state,
                    axis,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (remaining, None) | (None, remaining) => remaining,
            },
        }
    }
}

struct Tab {
    id: u64,
    active: u64,
    root: PaneTree,
}

/// Owns PTY entities independently of which tab or collapsed surface is rendered.
pub struct TerminalDeck {
    registry: TerminalState,
    owner: String,
    cwd: PathBuf,
    tabs: Vec<Tab>,
    active_tab: Option<u64>,
    focus_handle: FocusHandle,
    next_id: u64,
}

impl TerminalDeck {
    pub fn new(
        registry: TerminalState,
        owner: String,
        cwd: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut deck = Self {
            registry,
            owner,
            cwd,
            tabs: Vec::new(),
            active_tab: None,
            focus_handle: cx.focus_handle(),
            next_id: 1,
        };
        deck.new_tab(window, cx);
        deck
    }

    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.iter().find(|tab| Some(tab.id) == self.active_tab) {
            let pane = tab
                .root
                .pane(tab.active)
                .unwrap_or_else(|| tab.root.first());
            pane.panel.update(cx, |panel, cx| panel.focus(window, cx));
        } else {
            self.focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    fn count(&self) -> usize {
        self.tabs.iter().map(|tab| tab.root.count()).sum()
    }

    fn new_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Pane {
        let id = self.next_id;
        self.next_id += 1;
        let panel = cx.new(|cx| {
            TerminalPanel::new(
                self.registry.clone(),
                self.owner.clone(),
                self.cwd.clone(),
                window,
                cx,
            )
        });
        let handle = panel.read(cx).focus_handle(cx);
        let subscription = cx.on_focus(&handle, window, move |this, _, cx| {
            for tab in &mut this.tabs {
                if tab.root.pane(id).is_some() {
                    tab.active = id;
                    this.active_tab = Some(tab.id);
                    break;
                }
            }
            cx.notify();
        });
        let title = cx.subscribe(&panel, |_, _, _: &TerminalPanelEvent, cx| cx.notify());
        Pane {
            id,
            panel,
            _focus: subscription,
            _title: title,
        }
    }

    fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.count() >= MAX_PANES {
            return;
        }
        let pane = self.new_pane(window, cx);
        let id = pane.id;
        self.tabs.push(Tab {
            id,
            active: id,
            root: PaneTree::Leaf(pane),
        });
        self.active_tab = Some(id);
        self.focus(window, cx);
    }

    fn cycle_tab(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let current = self
            .tabs
            .iter()
            .position(|tab| Some(tab.id) == self.active_tab);
        if let Some(index) = cycled_index(current, self.tabs.len(), forward) {
            self.active_tab = Some(self.tabs[index].id);
        }
        self.focus(window, cx);
    }

    fn close_active_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.iter().find(|tab| Some(tab.id) == self.active_tab) {
            self.close_pane(tab.active, window, cx);
        }
    }

    fn split(&mut self, axis: Axis, window: &mut Window, cx: &mut Context<Self>) {
        if self.count() >= MAX_PANES {
            return;
        }
        let Some(index) = self
            .tabs
            .iter()
            .position(|tab| Some(tab.id) == self.active_tab)
        else {
            return;
        };
        let pane = self.new_pane(window, cx);
        let id = pane.id;
        let mut tab = self.tabs.remove(index);
        let state = cx.new(|_| ResizableState::default());
        tab.root = tab.root.split(tab.active, axis, pane, state);
        tab.active = id;
        self.tabs.insert(index, tab);
        self.focus(window, cx);
    }

    fn close_tab(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        self.tabs.remove(index);
        if self.active_tab == Some(id) {
            self.active_tab = self
                .tabs
                .get(index.min(self.tabs.len().saturating_sub(1)))
                .map(|tab| tab.id);
        }
        self.focus(window, cx);
    }

    fn close_pane(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|tab| tab.root.pane(id).is_some()) else {
            return;
        };
        let mut tab = self.tabs.remove(index);
        if let Some(root) = tab.root.remove(id) {
            if tab.active == id {
                tab.active = root.first().id;
            }
            tab.root = root;
            self.tabs.insert(index, tab);
        } else {
            self.active_tab = self
                .tabs
                .get(index.min(self.tabs.len().saturating_sub(1)))
                .map(|tab| tab.id);
        }
        self.focus(window, cx);
    }

    fn render_tree(
        &self,
        tree: &PaneTree,
        active: u64,
        split: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        match tree {
            PaneTree::Leaf(pane) => {
                let id = pane.id;
                let title = pane
                    .panel
                    .read(cx)
                    .title()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("终端 {id}"));
                div()
                    .id(("terminal-pane", id))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .when(split, |view| {
                        view.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .h(px(25.))
                                .flex_none()
                                .px_2()
                                .text_xs()
                                .text_color(if id == active {
                                    theme.foreground
                                } else {
                                    theme.muted_foreground
                                })
                                .child(div().flex_1().min_w_0().truncate().child(title))
                                .child(
                                    Button::new(("close-pane", id))
                                        .ghost()
                                        .xsmall()
                                        .icon(Keyline::Close.icon())
                                        .tooltip("关闭此分屏")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.close_pane(id, window, cx)
                                        })),
                                ),
                        )
                    })
                    .child(pane.panel.clone())
                    .into_any_element()
            }
            PaneTree::Split {
                id,
                state,
                axis,
                first,
                second,
            } => {
                let group = match axis {
                    Axis::Columns => h_resizable(("terminal-split", *id)),
                    Axis::Rows => v_resizable(("terminal-split", *id)),
                };
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .child(
                        group
                            .with_state(state)
                            .child(
                                resizable_panel()
                                    .size_range(px(40.)..Pixels::MAX)
                                    .child(self.render_tree(first, active, true, cx)),
                            )
                            .child(
                                resizable_panel()
                                    .size_range(px(40.)..Pixels::MAX)
                                    .child(self.render_tree(second, active, true, cx)),
                            ),
                    )
                    .into_any_element()
            }
        }
    }
}

impl Render for TerminalDeck {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let full = self.count() >= MAX_PANES;
        let empty = self.tabs.is_empty();
        let tabs = self
            .tabs
            .iter()
            .map(|tab| {
                let id = tab.id;
                let active = self.active_tab == Some(id);
                let name = self.cwd.file_name().unwrap_or_default().to_string_lossy();
                let title = tab
                    .root
                    .pane(tab.active)
                    .unwrap_or_else(|| tab.root.first())
                    .panel
                    .read(cx)
                    .title()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{name} · {id}"));
                div()
                    .id(("terminal-tab", id))
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(28.))
                    .px_1()
                    .rounded_md()
                    .flex_none()
                    .when(active, |view| view.bg(theme.secondary))
                    .child(
                        Button::new(("select-terminal", id))
                            .ghost()
                            .xsmall()
                            .max_w(px(180.))
                            .overflow_hidden()
                            .icon(Keyline::Terminal.icon())
                            .label(title)
                            .tooltip(self.cwd.to_string_lossy().into_owned())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.active_tab = Some(id);
                                this.focus(window, cx);
                            })),
                    )
                    .child(
                        Button::new(("close-terminal", id))
                            .ghost()
                            .xsmall()
                            .icon(Keyline::Close.icon())
                            .tooltip("关闭标签及其分屏")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.close_tab(id, window, cx)
                            })),
                    )
            })
            .collect::<Vec<_>>();
        let body = self
            .tabs
            .iter()
            .find(|tab| Some(tab.id) == self.active_tab)
            .map(|tab| self.render_tree(&tab.root, tab.active, false, cx));
        div()
            .id("terminal-deck")
            .key_context("TerminalDeck")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &NewTerminalTab, window, cx| {
                this.new_tab(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseTerminalPane, window, cx| {
                this.close_active_pane(window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextTerminalTab, window, cx| {
                this.cycle_tab(true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &PreviousTerminalTab, window, cx| {
                this.cycle_tab(false, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SplitTerminalColumns, window, cx| {
                this.split(Axis::Columns, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SplitTerminalRows, window, cx| {
                this.split(Axis::Rows, window, cx);
            }))
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(38.))
                    .flex_none()
                    .px_2()
                    .gap_1()
                    .child(
                        div()
                            .id("terminal-tabs")
                            .flex()
                            .items_center()
                            .gap_1()
                            .flex_1()
                            .min_w_0()
                            .overflow_x_scroll()
                            .children(tabs),
                    )
                    .child(
                        div()
                            .max_w(px(240.))
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(self.cwd.to_string_lossy().into_owned()),
                    )
                    .child(
                        Button::new("new-terminal")
                            .ghost()
                            .small()
                            .icon(Keyline::Plus.icon())
                            .tooltip_with_action(
                                if full {
                                    "最多 8 个终端"
                                } else {
                                    "新建终端"
                                },
                                &NewTerminalTab,
                                Some("TerminalDeck"),
                            )
                            .disabled(full)
                            .on_click(cx.listener(|this, _, window, cx| this.new_tab(window, cx))),
                    )
                    .child(
                        Button::new("split-terminal-columns")
                            .ghost()
                            .small()
                            .icon(Keyline::SplitColumns.icon())
                            .tooltip_with_action(
                                "左右分屏",
                                &SplitTerminalColumns,
                                Some("TerminalDeck"),
                            )
                            .disabled(full || empty)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.split(Axis::Columns, window, cx)
                            })),
                    )
                    .child(
                        Button::new("split-terminal-rows")
                            .ghost()
                            .small()
                            .icon(Keyline::SplitRows.icon())
                            .tooltip_with_action(
                                "上下分屏",
                                &SplitTerminalRows,
                                Some("TerminalDeck"),
                            )
                            .disabled(full || empty)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.split(Axis::Rows, window, cx)
                            })),
                    )
                    .child(
                        Button::new("minimize-terminal")
                            .ghost()
                            .small()
                            .icon(Keyline::Minus.icon())
                            .tooltip("隐藏终端（保留进程）")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(TerminalDeckEvent::Hide))),
                    ),
            )
            .children(body)
            .when(empty, |view| {
                view.child(
                    div()
                        .flex_1()
                        .p_3()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child("暂无终端，点击 + 新建"),
                )
            })
    }
}

impl EventEmitter<TerminalDeckEvent> for TerminalDeck {}

#[cfg(test)]
mod tests {
    use super::cycled_index;

    #[test]
    fn tab_navigation_wraps_and_handles_empty_or_stale_selection() {
        assert_eq!(cycled_index(None, 0, true), None);
        assert_eq!(cycled_index(None, 3, true), Some(0));
        assert_eq!(cycled_index(Some(9), 3, false), Some(0));
        assert_eq!(cycled_index(Some(2), 3, true), Some(0));
        assert_eq!(cycled_index(Some(0), 3, false), Some(2));
        assert_eq!(cycled_index(Some(1), 3, true), Some(2));
        assert_eq!(cycled_index(Some(1), 3, false), Some(0));
        assert_eq!(cycled_index(Some(0), 1, false), Some(0));
    }
}

#[cfg(all(test, feature = "test-support", unix))]
#[path = "deck_tests.rs"]
mod ui_tests;
