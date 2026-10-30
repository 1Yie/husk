use std::{path::Path, sync::Arc};

use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;

use crate::bridge::{ActivityCommand, Command, HostBridge, SessionKey};
use crate::ui::{icons::Keyline, popover::Popover};

/// Retained by the chat panel; opening the popover is the only automatic refresh.
pub(super) struct WorkspaceLinks {
    key: SessionKey,
    bridge: Arc<HostBridge>,
    roots: Vec<Value>,
    persist: bool,
    pending: Option<u64>,
    picking: bool,
    error: Option<String>,
}

impl WorkspaceLinks {
    pub(super) fn new(
        key: SessionKey,
        bridge: Arc<HostBridge>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            key,
            bridge,
            roots: Vec::new(),
            persist: false,
            pending: None,
            picking: false,
            error: None,
        }
    }

    fn busy(&self) -> bool {
        self.pending.is_some() || self.picking
    }

    fn send(&mut self, command: ActivityCommand, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        self.error = None;
        match self.bridge.send(Command::Activity(command)) {
            Ok(request) => self.pending = Some(request),
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    pub(super) fn receive(
        &mut self,
        request: u64,
        key: &SessionKey,
        value: &Value,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.pending != Some(request)
            || key != &self.key
            || value["kind"].as_str() != Some("workspace_links")
        {
            return false;
        }
        self.pending = None;
        if let Some(roots) = value["roots"].as_array() {
            self.roots = roots.clone();
            self.error = None;
        } else {
            self.error = Some("关联目录返回了无效的数据".into());
        }
        cx.notify();
        true
    }

    pub(super) fn request_failed(
        &mut self,
        request: u64,
        message: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.pending != Some(request) {
            return false;
        }
        self.pending = None;
        self.error = Some(message.to_owned());
        cx.notify();
        true
    }

    pub(super) fn control(entity: &Entity<Self>, cx: &App) -> Popover {
        let state = entity.read(cx);
        let content = entity.clone();
        let opened = entity.downgrade();
        Popover::new(SharedString::from(format!(
            "workspace-links:{}:{}",
            state.key.root, state.key.id
        )))
        .anchor(Anchor::BottomLeft)
        .offset(px(4.))
        .duration(crate::ui::motion::MENU_DURATION)
        .w(px(352.))
        .p(px(4.))
        .rounded(px(18.))
        .disabled(state.key.id <= 0)
        .trigger(
            Button::new("workspace-links-trigger")
                .ghost()
                .small()
                .h(px(28.))
                .px(px(10.))
                .rounded(px(14.))
                .text_size(px(12.))
                .child(
                    h_flex()
                        .gap(px(6.))
                        .child(Keyline::Copy.icon().size(px(14.)))
                        .child("关联工作区")
                        .child(Keyline::ChevronDown.icon().size(px(12.))),
                ),
        )
        .on_open_change(move |open, _, cx| {
            if *open {
                let _ = opened.update(cx, |state, cx| {
                    state.send(
                        ActivityCommand::WorkspaceLinks {
                            key: state.key.clone(),
                        },
                        cx,
                    );
                });
            }
        })
        .content(move |_, _, _| content)
    }

    fn choose_directory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择关联目录".into()),
        });
        let persist = self.persist;
        self.picking = true;
        self.error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = prompt.await;
            let _ = this.update_in(cx, |state, _, cx| {
                state.picking = false;
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            state.send(
                                ActivityCommand::LinkWorkspace {
                                    key: state.key.clone(),
                                    path: path.to_string_lossy().into_owned(),
                                    access: "readwrite".into(),
                                    persist,
                                },
                                cx,
                            );
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => state.error = Some(error.to_string()),
                    Err(error) => state.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn row(&self, index: usize, root: &Value, cx: &mut Context<Self>) -> AnyElement {
        let path = root["path"].as_str().unwrap_or("").to_owned();
        let name = Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        let scopes = root["scopes"].as_array();
        let workspace = scopes.is_some_and(|scopes| scopes.iter().any(|s| s == "workspace"));
        let session = scopes.is_some_and(|scopes| scopes.iter().any(|s| s == "session"));
        let scope = if session {
            "本会话"
        } else if workspace {
            "本工作区"
        } else {
            "本会话"
        };
        let writable = matches!(root["access"].as_str(), Some("readwrite" | "rw"));
        let busy = self.busy();
        let muted = cx.theme().muted_foreground;
        let foreground = cx.theme().foreground;
        let mut access = h_flex()
            .flex_shrink_0()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(14.))
            .bg(crate::ui::composer::alpha(
                if cx.theme().is_dark() {
                    0xfafafa
                } else {
                    0x09090b
                },
                0.06,
            ));
        for (label, value, selected) in
            [("只读", "read", !writable), ("读写", "readwrite", writable)]
        {
            let target = path.clone();
            access = access.child(
                Button::new(SharedString::from(format!("link-access-{index}-{value}")))
                    .ghost()
                    .small()
                    .h(px(24.))
                    .px(px(6.))
                    .rounded(px(12.))
                    .text_size(px(10.))
                    .text_color(if selected { foreground } else { muted })
                    .when(selected, |button| {
                        button.bg(rgb(if cx.theme().is_dark() {
                            0x232329
                        } else {
                            0xffffff
                        }))
                    })
                    .disabled(busy)
                    .label(label)
                    .on_click(cx.listener(move |state, _, _, cx| {
                        state.send(
                            ActivityCommand::SetWorkspaceAccess {
                                key: state.key.clone(),
                                path: target.clone(),
                                access: value.into(),
                            },
                            cx,
                        );
                    })),
            );
        }
        h_flex()
            .h(px(36.))
            .px(px(8.))
            .gap(px(6.))
            .child(
                div()
                    .max_w(px(84.))
                    .flex_shrink_0()
                    .truncate()
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(name),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .px(px(4.))
                    .rounded(px(4.))
                    .bg(crate::ui::composer::alpha(
                        if cx.theme().is_dark() {
                            0xfafafa
                        } else {
                            0x09090b
                        },
                        0.06,
                    ))
                    .text_color(muted)
                    .text_size(px(10.))
                    .child(scope),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(10.))
                    .text_color(rgb(if cx.theme().is_dark() {
                        0x55555f
                    } else {
                        0xa1a1aa
                    }))
                    .child(path.clone()),
            )
            .child(access)
            .child(
                Button::new(SharedString::from(format!("unlink-{index}")))
                    .ghost()
                    .small()
                    .w(px(16.))
                    .h(px(16.))
                    .p_0()
                    .rounded(px(4.))
                    .disabled(busy)
                    .tooltip(format!("取消关联 {path}"))
                    .child(Keyline::Close.icon().size(px(12.)).text_color(muted))
                    .on_click(cx.listener(move |state, _, _, cx| {
                        state.send(
                            ActivityCommand::UnlinkWorkspace {
                                key: state.key.clone(),
                                path: path.clone(),
                            },
                            cx,
                        );
                    })),
            )
            .into_any_element()
    }
}

impl Render for WorkspaceLinks {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.busy();
        let muted = cx.theme().muted_foreground;
        let mut rows = v_flex()
            .id("workspace-links-rows")
            .max_h(px(288.))
            .overflow_y_scroll();
        for (index, root) in self.roots.iter().enumerate() {
            rows = rows.child(self.row(index, root, cx));
        }
        let mut scope =
            h_flex()
                .p(px(2.))
                .gap(px(2.))
                .rounded_full()
                .bg(crate::ui::composer::alpha(
                    if cx.theme().is_dark() {
                        0xfafafa
                    } else {
                        0x09090b
                    },
                    0.06,
                ));
        for (label, persist) in [("本会话", false), ("本工作区", true)] {
            scope = scope.child(
                Button::new(if persist {
                    "link-scope-workspace"
                } else {
                    "link-scope-session"
                })
                .ghost()
                .small()
                .h(px(24.))
                .px(px(8.))
                .rounded(px(12.))
                .text_size(px(10.))
                .text_color(if self.persist == persist {
                    cx.theme().foreground
                } else {
                    muted
                })
                .when(self.persist == persist, |button| {
                    button.bg(rgb(if cx.theme().is_dark() {
                        0x232329
                    } else {
                        0xffffff
                    }))
                })
                .disabled(busy)
                .label(label)
                .on_click(cx.listener(move |state, _, _, cx| {
                    state.persist = persist;
                    cx.notify();
                })),
            );
        }
        v_flex()
            .w_full()
            .text_size(px(12.))
            .line_height(px(20.))
            .child(
                h_flex()
                    .h(px(28.))
                    .px(px(8.))
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .text_color(muted)
                    .child("关联工作区"),
            )
            .child(
                div()
                    .h(px(1.))
                    .mx(px(-4.))
                    .my(px(4.))
                    .bg(rgb(if cx.theme().is_dark() {
                        0x34343d
                    } else {
                        0xf4f4f5
                    })),
            )
            .when(self.roots.is_empty(), |panel| {
                panel.child(
                    div()
                        .px(px(8.))
                        .py(px(6.))
                        .text_size(px(11.))
                        .line_height(px(20.))
                        .text_color(muted)
                        .child(if self.pending.is_some() {
                            "正在加载关联目录…"
                        } else {
                            "还没有关联目录。跨仓库的项目可以在这里把另一个仓库加进来。"
                        }),
                )
            })
            .child(rows)
            .when_some(self.error.clone(), |panel, error| {
                panel.child(
                    div()
                        .px(px(12.))
                        .py(px(6.))
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            .child(
                div()
                    .h(px(1.))
                    .mx(px(-4.))
                    .my(px(4.))
                    .bg(rgb(if cx.theme().is_dark() {
                        0x34343d
                    } else {
                        0xf4f4f5
                    })),
            )
            .child(
                h_flex()
                    .h(px(40.))
                    .px(px(8.))
                    .justify_between()
                    .child(
                        Button::new("link-add-directory")
                            .ghost()
                            .small()
                            .h(px(24.))
                            .px(px(6.))
                            .text_size(px(12.))
                            .line_height(px(16.))
                            .disabled(busy)
                            .child(
                                h_flex()
                                    .gap(px(6.))
                                    .child(Keyline::Plus.icon().size(px(14.)))
                                    .child("添加目录"),
                            )
                            .on_click(cx.listener(|state, _, window, cx| {
                                state.choose_directory(window, cx);
                            })),
                    )
                    .child(scope),
            )
    }
}
