use super::*;
use gpui_kit::base::motion::{Easing, MotionReveal, Presence, Transition};
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    dialog::DialogEntrance,
    h_flex,
    input::Input,
    menu::PopupMenuItem,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;

use crate::ui::{icons::Keyline, popover::Menu};

impl ActivityPanel {
    fn artifact_row(&self, row: &Value, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let path = field(row, "path").to_owned();
        let open_path = path.clone();
        let selected = self.detail == Some(Detail::Artifact(path.clone()));
        v_flex()
            .w_full()
            .min_w_0()
            .py_2()
            .px_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new(SharedString::from(format!("artifact-{index}")))
                    .ghost()
                    .small()
                    .w_full()
                    .min_w_0()
                    .justify_start()
                    .icon(Keyline::File.icon())
                    .label(path.clone())
                    .tooltip(format!("安全文本预览：{path}"))
                    .when(selected, |b| b.bg(cx.theme().secondary))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.load_detail(Detail::Artifact(open_path.clone()), true, cx);
                    })),
            )
            .child(
                div()
                    .px_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{} · 历史记录 #{}",
                        field(row, "tool"),
                        row["index"]
                    )),
            )
            .into_any_element()
    }

    fn job_row(&self, row: &Value, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let id = field(row, "id").to_owned();
        let kind = field(&row["state"], "kind");
        let paused = kind == "paused";
        let live = matches!(kind, "running" | "paused");
        let restored = row["restored"].as_bool().unwrap_or(false);
        let menu_id = id.clone();
        let shell = cx.weak_entity();
        let state = match kind {
            "running" => "运行中",
            "paused" => "已暂停",
            "done" => field(&row["state"], "label"),
            _ => "状态未知",
        };
        v_flex()
            .w_full()
            .min_w_0()
            .py_2()
            .px_3()
            .gap_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .child(field(row, "command").to_owned()),
                    )
                    .child(Menu::new(
                        SharedString::from(format!("job-menu-{index}")),
                        Button::new("job-actions")
                            .ghost()
                            .small()
                            .icon(Keyline::More.icon())
                            .tooltip("后台任务操作"),
                        move |mut menu, _, _| {
                            if live && !restored {
                                let pause_shell = shell.clone();
                                let job = menu_id.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(if paused { "恢复" } else { "暂停" })
                                        .on_click(move |_, _, cx| {
                                            let _ = pause_shell.update(cx, |this, cx| {
                                                this.job_control(
                                                    job.clone(),
                                                    if paused { "resume" } else { "pause" },
                                                    cx,
                                                )
                                            });
                                        }),
                                );
                                let kill_shell = shell.clone();
                                let job = menu_id.clone();
                                menu = menu.item(PopupMenuItem::new("终止").on_click(
                                    move |_, window, cx| {
                                        let weak = kill_shell.clone();
                                        let job = job.clone();
                                        window.open_alert_dialog(cx, move |dialog, _, _| {
                                            let weak = weak.clone();
                                            let job = job.clone();
                                            dialog
                                                .confirm()
                                                .entrance(DialogEntrance::FadeSlide)
                                                .title("终止后台任务？")
                                                .description(
                                                    "任务进程将被终止，已产生的输出仍保留。",
                                                )
                                                .on_ok(move |_, _, cx| {
                                                    let _ = weak.update(cx, |this, cx| {
                                                        this.job_control(job.clone(), "kill", cx)
                                                    });
                                                    true
                                                })
                                        });
                                    },
                                ));
                            } else {
                                let job = menu_id.clone();
                                menu = menu.item(PopupMenuItem::new("移除记录").on_click(
                                    move |_, _, cx| {
                                        let _ = shell.update(cx, |this, cx| {
                                            this.job_control(job.clone(), "remove", cx)
                                        });
                                    },
                                ));
                            }
                            menu
                        },
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(div().flex_1().min_w_0().truncate().child(format!(
                        "{id} · {state}{}",
                        if restored { " · 保存的记录" } else { "" }
                    )))
                    .child(
                        Button::new(SharedString::from(format!("logs-{index}")))
                            .ghost()
                            .small()
                            .label("日志")
                            .disabled(restored && row["log_tail"].as_str().is_none())
                            .tooltip(if restored {
                                "查看保存的日志末尾"
                            } else {
                                "读取最后 200 行日志"
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.load_detail(Detail::Logs(id.clone()), true, cx)
                            })),
                    ),
            )
            .into_any_element()
    }

    fn child_row(&self, row: &Value, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(id) = row["child_id"].as_u64() else {
            return div().into_any_element();
        };
        let waiting = row["waiting"].as_str().filter(|s| !s.is_empty());
        let state = if waiting.is_some() {
            "等待输入"
        } else if field(row, "state") == "running" {
            "运行中"
        } else {
            "已保存转录"
        };
        v_flex()
            .w_full()
            .min_w_0()
            .px_3()
            .py_2()
            .gap_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new(SharedString::from(format!("child-{index}")))
                    .ghost()
                    .small()
                    .w_full()
                    .justify_start()
                    .label(format!("{} · {state}", field(row, "label")))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.load_detail(Detail::Child(id), true, cx)
                    })),
            )
            .when(!field(row, "task").is_empty(), |view| {
                view.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(field(row, "task").to_owned()),
                )
            })
            .when_some(waiting, |view, waiting| {
                view.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(waiting.to_owned()),
                )
            })
            .into_any_element()
    }

    fn detail_body(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(detail) = self.detail.clone() else {
            return div().into_any_element();
        };
        let title = match &detail {
            Detail::Artifact(path) => path.clone(),
            Detail::Logs(job) => format!("日志 · {job}"),
            Detail::Child(id) => format!("子代理 {id} · 只读转录"),
        };
        let copy = self.detail_text.clone();
        let expand_text = self.detail_text.clone();
        let expand_title = title.clone();
        let child = match detail {
            Detail::Child(id) => Some(id),
            _ => None,
        };
        let running_child = child.is_some_and(|id| {
            self.children
                .iter()
                .any(|row| row["child_id"].as_u64() == Some(id) && field(row, "state") == "running")
        });
        let _ = window;
        v_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .p_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_1()
                    .w_full()
                    .min_w_0()
                    .child(div().text_sm().flex_1().min_w_0().truncate().child(title))
                    .child(
                        Button::new("activity-copy")
                            .ghost()
                            .small()
                            .label("复制")
                            .disabled(self.detail_text.is_empty())
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                            }),
                    )
                    .child(
                        Button::new("activity-expand")
                            .ghost()
                            .small()
                            .label("展开")
                            .disabled(self.detail_text.is_empty())
                            .on_click(move |_, window, cx| {
                                let html = format!(
                                    "<pre>{}</pre>",
                                    escape(&bounded(&expand_text, 500_000))
                                );
                                let state = cx.new(|cx| TextViewState::html(&html, cx));
                                let title = expand_title.clone();
                                window.open_dialog(cx, move |dialog, _, _| {
                                    dialog
                                        .entrance(DialogEntrance::FadeSlide)
                                        .title(title.clone())
                                        .width(px(1000.))
                                        .child(
                                            div()
                                                .id("activity-expanded-text")
                                                .h(px(600.))
                                                .overflow_y_scroll()
                                                .child(
                                                    TextView::new(&state)
                                                        .selectable(true)
                                                        .stream_fade(false),
                                                ),
                                        )
                                });
                            }),
                    )
                    .child(
                        Button::new("activity-close")
                            .ghost()
                            .small()
                            .icon(Keyline::Close.icon())
                            .tooltip("关闭详情")
                            .on_click(cx.listener(|this, _, _, cx| this.close_detail(cx))),
                    ),
            )
            .when(self.detail_loading, |view| {
                view.child(status("正在读取…", cx))
            })
            .when_some(self.detail_error.as_ref(), |view, error| {
                view.child(status(error.clone(), cx))
            })
            .when(
                !self.detail_loading && self.detail_error.is_none() && self.detail_text.is_empty(),
                |view| {
                    view.child(status(
                        "暂无可读取的内容。运行中的子代理可能尚未保存转录。",
                        cx,
                    ))
                },
            )
            .when_some(self.detail_view.clone(), |view, text| {
                view.child(
                    div()
                        .id("activity-detail-content")
                        .w_full()
                        .max_h(px(340.))
                        .overflow_y_scroll()
                        .text_size(px(12.))
                        .child(TextView::new(&text).selectable(true).stream_fade(false)),
                )
            })
            .when(running_child, |view| {
                view.child(
                    v_flex().gap_2().child(Input::new(&self.steer)).child(
                        Button::new("activity-steer")
                            .small()
                            .label("发送指导")
                            .disabled(
                                self.latest
                                    .get("steer")
                                    .is_some_and(|id| self.pending.contains_key(id)),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(id) = child {
                                    this.steer_child(id, cx)
                                }
                            })),
                    ),
                )
            })
            .into_any_element()
    }
}

fn status(text: impl AsRef<str>, cx: &App) -> AnyElement {
    div()
        .px_3()
        .py_2()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.as_ref().to_owned())
        .into_any_element()
}

impl Render for ActivityPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, empty, count, source_error) = match self.tab {
            ActivityTab::Artifacts => (
                "产物",
                "当前会话尚无成功写入的产物。",
                self.artifacts.len(),
                self.artifact_error.clone(),
            ),
            ActivityTab::Jobs => ("后台任务", "当前会话没有后台任务。", self.jobs.len(), None),
            ActivityTab::Subagents => (
                "子代理",
                "当前会话没有运行中的子代理或已保存的转录。",
                self.children.len(),
                self.child_error.clone(),
            ),
        };
        let mut list = v_flex().w_full().min_w_0();
        match self.tab {
            ActivityTab::Artifacts => {
                for (i, row) in self.artifacts.iter().enumerate() {
                    list = list.child(self.artifact_row(row, i, cx))
                }
            }
            ActivityTab::Jobs => {
                for (i, row) in self.jobs.iter().enumerate() {
                    list = list.child(self.job_row(row, i, cx))
                }
            }
            ActivityTab::Subagents => {
                for (i, row) in self.children.iter().enumerate() {
                    list = list.child(self.child_row(row, i, cx))
                }
            }
        }
        let detail = self.detail_body(window, cx);
        let presence = Presence::new("activity-detail-presence", self.detail.is_some())
            .transition(Transition::new(Duration::from_millis(160)).easing(Easing::EaseOut))
            .sample(window, cx);
        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .text_color(cx.theme().foreground)
            .when(self.header_visible, |view| {
                view.child(
                    h_flex()
                        .px_3()
                        .py_1()
                        .gap_2()
                        .flex_shrink_0()
                        .child(div().text_xs().flex_1().child(format!("{title} · {count}")))
                        .child(
                            Button::new("activity-refresh")
                                .ghost()
                                .small()
                                .label(if self.loading {
                                    "刷新中…"
                                } else {
                                    "刷新"
                                })
                                .disabled(self.session.is_none() || self.loading)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.refresh(window, cx)),
                                ),
                        ),
                )
            })
            .when(self.session.is_none(), |view| {
                view.child(status("选择会话以查看活动。", cx))
            })
            .when_some(self.error.as_ref(), |view, error| {
                view.child(status(format!("操作失败：{error}"), cx))
            })
            .when_some(source_error.as_ref(), |view, error| {
                view.child(status(format!("读取失败：{error}"), cx))
            })
            .when_some(self.notice.as_ref(), |view, notice| {
                view.child(status(notice.clone(), cx))
            })
            .when(self.loading && !self.loaded, |view| {
                view.child(status("正在读取会话活动…", cx))
            })
            .when(
                self.session.is_some() && self.loaded && count == 0 && source_error.is_none(),
                |view| view.child(status(empty, cx)),
            )
            .child(
                div()
                    .id("activity-scroll")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(list)
                    .when(presence.should_render(), |view| {
                        view.child(MotionReveal::new(
                            "activity-detail-reveal",
                            presence.progress,
                            detail,
                        ))
                    }),
            )
    }
}
