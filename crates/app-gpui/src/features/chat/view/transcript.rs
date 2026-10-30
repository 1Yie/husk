use super::*;
use crate::features::chat::turns::Phase;
use gpui_kit::{
    base::motion::{Easing, MotionReveal, MotionStatus, Transition, transition_with_status},
    component::{WindowExt, dialog::DialogEntrance},
};

fn literal_text(text: &str) -> String {
    format!(
        "<pre>{}</pre>",
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    )
}

impl ChatPanel {
    pub(super) fn render_turn(
        &mut self,
        turn_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(turn) = self.turns.get(turn_index).cloned() else {
            return div().into_any_element();
        };
        self.scroller
            .read(cx)
            .revealing
            .borrow_mut()
            .remove(&turn_index);
        let theme = cx.theme().clone();
        let wheel_scroller = self.scroller.clone();
        let mut body = v_flex()
            .id(("turn", turn.id))
            .w_full()
            .min_w_0()
            .max_w(px(920.))
            .mx_auto()
            .pl(px(50.))
            .pr(px(24.))
            .pb(px(32.))
            .gap_3()
            .on_scroll(move |event, _, cx| {
                if wheel_scroller.update(cx, |state, cx| state.smooth_wheel(event, cx)) {
                    cx.stop_propagation();
                }
            });
        for phase in turn.phases(&self.model.entries) {
            match phase {
                Phase::Entry(index) => {
                    body = body.child(self.render_turn_entry(index, turn_index, window, cx));
                }
                Phase::Tools(range) => {
                    let first = self.model.entries[range.start].clone();
                    let live = self.model.running
                        && self.model.entries[range.clone()]
                            .iter()
                            .any(|entry| !entry.complete);
                    let label = if live {
                        "正在调用工具".to_owned()
                    } else {
                        format!("{} 次工具调用", range.len())
                    };
                    let preview = self.model.entries[range.clone()]
                        .last()
                        .map(|entry| format!("{} {}", entry.title, entry.detail))
                        .unwrap_or_default();
                    let header = self.phase_header(first.id, &label, &preview, turn_index, cx);
                    let progress = self.reveal_progress(first.id, 300, turn_index, window, cx);
                    let mut tools = v_flex().w_full().min_w_0().gap_1().pl_3();
                    // Like Tauri RowDetail, never render expensive output until
                    // its containing phase has actually been opened.
                    if self.revealed.contains(&first.id) {
                        for index in range {
                            tools = tools.child(self.render_tool(index, turn_index, window, cx));
                        }
                    }
                    body = body.child(v_flex().w_full().min_w_0().gap_1().child(header).child(
                        MotionReveal::new(
                            ("tools-reveal", first.id),
                            progress,
                            tools.opacity(progress).into_any_element(),
                        ),
                    ));
                }
            }
        }
        let live = self.model.running && turn_index + 1 == self.turns.len();
        let has_assistant = self.model.entries[turn.entries.clone()]
            .iter()
            .any(|entry| entry.kind != EntryKind::User);
        if live {
            // This wait row is not a fake message and cannot acquire the
            // identity of a streamed answer.
            body = body.child(div().text_xs().text_color(theme.muted_foreground).child(
                if self.model.status.is_empty() {
                    "正在回复".to_owned()
                } else {
                    self.model.status.clone()
                },
            ));
        } else if has_assistant {
            let copy = turn.copy_text(&self.model.entries);
            let id = turn.id;
            let mut footer = h_flex()
                .gap_1()
                .text_xs()
                .text_color(theme.muted_foreground);
            if !copy.trim().is_empty() {
                footer = footer.child(
                    Button::new(("copy-turn", id))
                        .ghost()
                        .small()
                        .label("复制")
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                        }),
                );
            }
            footer = footer.child(
                Button::new(("history-turn", id))
                    .ghost()
                    .small()
                    .label("原始消息")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_turn_history(id, window, cx)
                    })),
            );
            let elapsed = self
                .clocks
                .get(&id)
                .and_then(|(_, elapsed)| *elapsed)
                .or_else(|| turn.elapsed(&self.model.entries));
            if let Some(elapsed) = elapsed {
                footer = footer.child(format!("耗时 {}", turns::elapsed_label(elapsed)));
            }
            if let Some(time) = turn
                .assistant_timestamp(&self.model.entries)
                .and_then(turns::timestamp_label)
            {
                footer = footer.child(time);
            }
            body = body.child(footer);
        }
        body.into_any_element()
    }

    fn toggle_trace(&mut self, id: u64, turn_index: usize, cx: &mut Context<Self>) {
        if !self.expanded.remove(&id) {
            self.expanded.insert(id);
            self.revealed.insert(id);
        }
        self.scroller.update(cx, |state, cx| {
            state.list.pause_following_tail();
            state.remeasure_items(turn_index..turn_index + 1, cx);
        });
        cx.notify();
    }

    fn reveal_progress(
        &self,
        id: u64,
        ms: u64,
        turn_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> f32 {
        let motion = transition_with_status(
            (ElementId::from(("trace", id)), "reveal"),
            if self.expanded.contains(&id) {
                1.0f32
            } else {
                0.0
            },
            Transition::new(Duration::from_millis(ms)).easing(Easing::EaseOut),
            window,
            cx,
        );
        if matches!(motion.status, MotionStatus::Running | MotionStatus::Delayed) {
            self.scroller
                .read(cx)
                .revealing
                .borrow_mut()
                .insert(turn_index);
        }
        motion.value
    }

    fn phase_header(
        &self,
        id: u64,
        label: &str,
        preview: &str,
        turn_index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.expanded.contains(&id);
        let theme = cx.theme().clone();
        h_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .child(
                Button::new(("expand-phase", id))
                    .ghost()
                    .small()
                    .icon(if open {
                        Keyline::ChevronDown.icon()
                    } else {
                        Keyline::ChevronRight.icon()
                    })
                    .label(label.to_owned())
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.toggle_trace(id, turn_index, cx)),
                    ),
            )
            .when(!open && !preview.trim().is_empty(), |this| {
                this.child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(preview.to_owned()),
                )
            })
            .into_any_element()
    }

    fn render_turn_entry(
        &mut self,
        index: usize,
        turn_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entry = self.model.entries[index].clone();
        let theme = cx.theme().clone();
        match entry.kind {
            EntryKind::User => {
                let (text, attachments) = display_payload(&entry.text);
                let mut bubble = v_flex()
                    .min_w_0()
                    .gap_2()
                    .px(px(14.))
                    .py(px(10.))
                    .rounded(px(12.))
                    .bg(theme.muted)
                    .text_size(px(14.));
                if !text.is_empty() {
                    let mut displayed = entry.clone();
                    displayed.text = text;
                    bubble = bubble.child(self.markdown_view(&displayed, cx));
                }
                for attachment in attachments {
                    bubble = bubble.child(
                        h_flex()
                            .gap_1()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(Keyline::File.icon().size(px(14.)))
                            .child(format!(
                                "{}{}",
                                attachment.name,
                                if attachment.truncated {
                                    "（已截断）"
                                } else {
                                    ""
                                }
                            )),
                    );
                }
                let copy = display_payload(&entry.text).0;
                v_flex()
                    .w_full()
                    .items_end()
                    .gap_1()
                    .child(div().max_w(relative(0.8)).min_w_0().child(bubble))
                    .when(!copy.is_empty(), |this| {
                        this.child(
                            Button::new(("copy-user", entry.id))
                                .ghost()
                                .small()
                                .label("复制")
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                                }),
                        )
                    })
                    .when_some(
                        entry.timestamp.and_then(turns::timestamp_label),
                        |this, time| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(time),
                            )
                        },
                    )
                    .into_any_element()
            }
            EntryKind::Assistant => self.markdown_view(&entry, cx),
            EntryKind::Reasoning => {
                // A following phase settles thinking even if the host has not
                // yet emitted its final AssistantMessage.
                let live =
                    self.model.running && !entry.complete && index + 1 == self.model.entries.len();
                let mut label = if live {
                    "思考中".to_owned()
                } else {
                    "思考过程".to_owned()
                };
                if let Some(ms) = entry.duration_ms.filter(|ms| *ms >= 0) {
                    label.push_str(&format!(
                        " · {}",
                        turns::elapsed_label(Duration::from_millis(ms as u64))
                    ));
                }
                let header = self.phase_header(
                    entry.id,
                    &label,
                    entry
                        .text
                        .lines()
                        .rev()
                        .find(|line| !line.trim().is_empty())
                        .unwrap_or(""),
                    turn_index,
                    cx,
                );
                let progress = self.reveal_progress(entry.id, 300, turn_index, window, cx);
                let detail = if self.revealed.contains(&entry.id) {
                    // Reasoning is selectable markdown, but never mixed with
                    // the final-answer copy action.
                    self.markdown_view(&entry, cx)
                } else {
                    div().into_any_element()
                };
                v_flex()
                    .w_full()
                    .min_w_0()
                    .child(header)
                    .child(MotionReveal::new(
                        ("thinking-reveal", entry.id),
                        progress,
                        div()
                            .pl_3()
                            .text_color(theme.muted_foreground)
                            .opacity(progress)
                            .child(detail)
                            .into_any_element(),
                    ))
                    .into_any_element()
            }
            EntryKind::Error | EntryKind::Notice => v_flex()
                .w_full()
                .min_w_0()
                .gap_1()
                .text_sm()
                .text_color(if entry.kind == EntryKind::Error {
                    theme.danger
                } else {
                    theme.muted_foreground
                })
                .when(!entry.title.is_empty(), |this| this.child(entry.title))
                .child(entry.text)
                .into_any_element(),
            EntryKind::Tool | EntryKind::Child => self.render_tool(index, turn_index, window, cx),
        }
    }

    fn render_tool(
        &mut self,
        index: usize,
        turn_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entry = self.model.entries[index].clone();
        // A tools phase uses its first Entry's id; the nested row must have a
        // separate key or opening the group would also open the first result.
        let id = entry.id | (1 << 63);
        let open = self.expanded.contains(&id);
        let theme = cx.theme().clone();
        let status = if !entry.complete && self.model.running {
            "进行中"
        } else if entry.ok == Some(false) {
            "已中断"
        } else {
            "完成"
        };
        let has_detail = !entry.detail.is_empty() || !entry.text.is_empty();
        let header = h_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .child(
                Button::new(("expand-tool", entry.id))
                    .ghost()
                    .small()
                    .label(if entry.title.is_empty() {
                        entry.kind.label().to_owned()
                    } else {
                        entry.title.clone()
                    })
                    .icon(if open {
                        Keyline::ChevronDown.icon()
                    } else {
                        Keyline::ChevronRight.icon()
                    })
                    .disabled(!has_detail)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.toggle_trace(id, turn_index, cx)),
                    ),
            )
            .when(!entry.detail.is_empty(), |this| {
                this.child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .px_1()
                        .rounded_md()
                        .bg(theme.muted)
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(entry.detail.clone()),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(if entry.ok == Some(false) {
                        theme.danger
                    } else {
                        theme.muted_foreground
                    })
                    .child(status),
            );
        let progress = self.reveal_progress(id, 250, turn_index, window, cx);
        let detail = if self.revealed.contains(&id) {
            let output = if entry.detail.is_empty() {
                entry.text.clone()
            } else if entry.text.is_empty() {
                entry.detail.clone()
            } else {
                format!("{}\n\n{}", entry.detail, entry.text)
            };
            let output = literal_text(&output);
            let state = self.markdown.entry(id).or_insert_with(|| Markdown {
                text: output.clone(),
                state: cx.new(|cx| TextViewState::html(&output, cx)),
            });
            if state.text != output {
                state
                    .state
                    .update(cx, |state, cx| state.set_text(&output, cx));
                state.text = output;
            }
            div()
                .id(("tool-output", entry.id))
                .w_full()
                .max_h(px(400.))
                .overflow_y_scroll()
                .p_3()
                .rounded(px(12.))
                .border_1()
                .border_color(theme.border)
                .bg(theme.muted)
                .child(
                    TextView::new(&state.state)
                        .text_size(px(11.5))
                        .selectable(true),
                )
                .into_any_element()
        } else {
            div().into_any_element()
        };
        v_flex()
            .w_full()
            .min_w_0()
            .child(header)
            .child(MotionReveal::new(
                ("tool-reveal", entry.id),
                progress,
                div()
                    .w_full()
                    .py_1()
                    .opacity(progress)
                    .child(detail)
                    .into_any_element(),
            ))
            .into_any_element()
    }

    fn open_turn_history(&self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(turn) = self.turns.iter().find(|turn| turn.id == id) else {
            return;
        };
        let text = self.model.entries[turn.entries.clone()]
            .iter()
            .map(|entry| {
                format!(
                    "{}{}\n{}{}",
                    entry.kind.label(),
                    if entry.title.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", entry.title)
                    },
                    if entry.detail.is_empty() {
                        String::new()
                    } else {
                        format!("{}\n", entry.detail)
                    },
                    entry.text
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let text = literal_text(&text);
        let content = cx.new(|cx| TextViewState::html(&text, cx));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("原始消息")
                .entrance(DialogEntrance::FadeSlide)
                .width(px(760.))
                .child(
                    div()
                        .text_xs()
                        .child("本轮界面消息（不包含隐藏的系统消息）"),
                )
                .child(
                    div()
                        .id("turn-history-body")
                        .max_h(px(520.))
                        .overflow_y_scroll()
                        .child(TextView::new(&content).text_size(px(13.)).selectable(true)),
                )
        });
    }

    pub(super) fn turn_rail(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let active = self
            .scroller
            .read(cx)
            .list
            .logical_scroll_top()
            .item_ix
            .min(self.turns.len().saturating_sub(1));
        // Keep the rail bounded rather than mounting a button for every turn.
        let start = active
            .saturating_sub(12)
            .min(self.turns.len().saturating_sub(25));
        let mut rail = v_flex()
            .gap_1()
            .items_center()
            .justify_center()
            .px_1()
            .py_2()
            .rounded_full();
        rail = rail.child(
            Button::new("rail-top")
                .ghost()
                .small()
                .label("↑")
                .tooltip("顶部")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.scroller
                        .update(cx, |state, cx| state.jump_to(Some(0), cx))
                })),
        );
        if start > 0 {
            rail = rail.child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("~"),
            );
        }
        for (index, turn) in self.turns.iter().enumerate().skip(start).take(25) {
            let preview = self.model.entries[turn.entries.clone()]
                .iter()
                .find(|entry| entry.kind == EntryKind::User)
                .map(|entry| queued_label(&entry.text))
                .unwrap_or_else(|| "Husk".into());
            rail = rail.child(
                Button::new(("rail-turn", turn.id))
                    .ghost()
                    .small()
                    .w(px(24.))
                    .h(px(11.))
                    .label(if index == active { "━" } else { "─" })
                    .tooltip(preview)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.scroller
                            .update(cx, |state, cx| state.jump_to(Some(index), cx))
                    })),
            );
        }
        if start + 25 < self.turns.len() {
            rail = rail.child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("~"),
            );
        }
        rail.child(
            Button::new("rail-bottom")
                .ghost()
                .small()
                .label("↓")
                .tooltip("底部")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.scroller
                        .update(cx, |state, cx| state.jump_to(None, cx))
                })),
        )
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::literal_text;

    #[test]
    fn raw_tool_text_cannot_become_html_or_markup() {
        assert_eq!(
            literal_text("<script>& `code`</script>"),
            "<pre>&lt;script&gt;&amp; `code`&lt;/script&gt;</pre>"
        );
    }

    #[cfg(feature = "test-support")]
    #[gpui_kit::test]
    fn turn_headers_toggle_without_changing_stream_or_session_identity(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        const NAME: &str = "features::chat::view::transcript::tests::turn_headers_toggle_without_changing_stream_or_session_identity";
        if std::env::var("HUSK_GPUI_TURN_UI_TEST").as_deref() != Ok(NAME) {
            let isolated = tempfile::tempdir().unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .current_dir(isolated.path())
                .args(["--exact", NAME, "--nocapture"])
                .env("HUSK_GPUI_TURN_UI_TEST", NAME)
                .env("AGENT_SESSION_DB", isolated.path().join("sessions"))
                .env("XDG_CONFIG_HOME", isolated.path().join("config"))
                .env("XDG_DATA_HOME", isolated.path().join("data"))
                .env("HOME", isolated.path().join("home"))
                .env("HUSK_GPUI_DISABLE_DESKTOP_NOTIFICATIONS", "1")
                .status()
                .unwrap();
            assert!(status.success(), "isolated turn window test failed");
            return;
        }
        use super::*;
        use gpui_kit::test::TestWindowExt;
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let paths =
            agent_paths::AppPaths::new(dir.path().join("config"), dir.path().join("data"), ".husk");
        let bridge = Arc::new(HostBridge::spawn(paths).unwrap());
        let key = SessionKey {
            root: workspace.to_string_lossy().into_owned(),
            id: 1,
        };
        cx.update(gpui_kit::init);
        let (window, panel) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| ChatPanel::new(key, bridge.clone(), &[], window, cx))
            })
            .unwrap()
        });
        cx.update_window(window.into(), |_, window, cx| {
            panel.update(cx, |panel, cx| {
                panel.apply_event(&agent_ipc::UiEvent::UserPrompt("task".into()), cx);
                panel.apply_event(
                    &agent_ipc::UiEvent::ReasoningDelta {
                        text: "thinking".into(),
                        parent: None,
                        child_id: None,
                    },
                    cx,
                );
                panel.apply_event(
                    &agent_ipc::UiEvent::ToolCallStarted {
                        name: "read".into(),
                        args_preview: "file.rs".into(),
                        parent: None,
                        child_id: None,
                    },
                    cx,
                );
                assert_eq!(panel.turns.len(), 1);
                assert_eq!(panel.scroller.read(cx).item_count(), 1);
            });
            window.render_frame(cx);
            assert!(window.try_find(("expand-tool", 3u64)).is_none());
            window.click(("expand-phase", 3u64), cx);
            panel.update(cx, |panel, cx| {
                assert!(panel.expanded.contains(&3));
                assert!(panel.revealed.contains(&3));
                let turn_id = panel.turns[0].id;
                panel.apply_event(
                    &agent_ipc::UiEvent::TextDelta {
                        text: "answer".into(),
                        parent: None,
                        child_id: None,
                    },
                    cx,
                );
                let answer_id = panel.model.entries.last().unwrap().id;
                panel.apply_event(&agent_ipc::UiEvent::AssistantMessage("final".into()), cx);
                assert_eq!(panel.turns[0].id, turn_id);
                assert_eq!(panel.model.entries.last().unwrap().id, answer_id);
                // A history replacement is a new presentation scope; neither
                // fold state nor old Markdown entities may leak into it.
                panel.replace_history(&[agent_llm::types::ChatMessage::user("other")], cx);
                assert!(panel.expanded.is_empty());
                assert!(panel.revealed.is_empty());
                assert!(panel.markdown.is_empty());
            });
            window.render_frame(cx);
        })
        .unwrap();
        bridge.shutdown();
    }
}
