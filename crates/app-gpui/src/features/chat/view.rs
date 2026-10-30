use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{InputEvent, Textarea, TextareaState},
    menu::PopupMenuItem,
    text::{TextView, TextViewState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::attachments::{Attachment, EditablePayload, build_payload, display_payload};
use super::model::{ChatModel, Entry, EntryKind};
use super::model_picker::ModelPicker;
use super::scroller::{self, TranscriptState};
use super::submission::{Draft, Submissions, merge_attachments};
use super::turns::{self, Turn};
use super::workspace_links::WorkspaceLinks;
use crate::bridge::{Command, ConfigInfo, HostBridge, SessionKey};
use crate::ui::icons::Keyline;
use crate::ui::layout;
use crate::ui::popover::Menu;

mod transcript;

struct Markdown {
    text: String,
    state: Entity<TextViewState>,
}

struct QueueEdit {
    index: usize,
    expected: Arc<Vec<String>>,
    payload: EditablePayload,
    input: Entity<TextareaState>,
    pending: Option<u64>,
    error: Option<String>,
    _subscription: Subscription,
}

fn fast_mode_state(
    current_model: Option<&serde_json::Value>,
    info: &serde_json::Value,
) -> Option<bool> {
    current_model
        .and_then(|model| model["supports_fast_mode"].as_bool())
        .filter(|supported| *supported)
        .map(|_| info["active_fast_mode"].as_bool().unwrap_or(false))
}

fn can_submit(text: &str, has_attachments: bool, answering: bool, loading: bool) -> bool {
    !loading && (!text.trim().is_empty() || (!answering && has_attachments))
}

fn permission_label(mode: &str) -> &'static str {
    match mode {
        "acceptEdits" => "接受编辑",
        "auto" => "自动",
        "dontAsk" => "不询问",
        "bypassPermissions" => "跳过权限",
        _ => "默认",
    }
}

fn queued_label(payload: &str) -> String {
    let (body, attachments) = display_payload(payload);
    let mut label = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if !attachments.is_empty() {
        if !label.is_empty() {
            label.push_str(" · ");
        }
        label.push_str(&format!("{} 个附件", attachments.len()));
    }
    if label.is_empty() {
        label.push_str("空消息");
    }
    let mut characters = label.chars().filter(|character| {
        !character.is_control()
            && !matches!(
                *character,
                '\u{061c}' | '\u{200e}'..='\u{200f}' | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
            )
    });
    let shortened: String = characters.by_ref().take(120).collect();
    if characters.next().is_some() {
        format!("{shortened}…")
    } else {
        shortened
    }
}

pub struct ChatPanel {
    key: SessionKey,
    bridge: Arc<HostBridge>,
    pub model: ChatModel,
    composer: Entity<TextareaState>,
    scroller: Entity<TranscriptState>,
    turns: Vec<Turn>,
    clocks: HashMap<u64, (Instant, Option<Duration>)>,
    markdown: HashMap<u64, Markdown>,
    expanded: HashSet<u64>,
    revealed: HashSet<u64>,
    submissions: Submissions,
    attachments: Vec<Attachment>,
    attachment_loading: bool,
    queue_edit: Option<QueueEdit>,
    error: Option<String>,
    model_info: serde_json::Value,
    model_picker: Entity<ModelPicker>,
    workspace_links: Entity<WorkspaceLinks>,
    config: Option<ConfigInfo>,
    _subscriptions: Vec<Subscription>,
}

impl ChatPanel {
    pub fn new(
        key: SessionKey,
        bridge: Arc<HostBridge>,
        history: &[agent_llm::types::ChatMessage],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let model = ChatModel::from_history(history);
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("输入消息… @ 引用文件 · / 命令 · $ 技能")
                .auto_grow(1, 8)
                .submit_on_enter(false)
        });
        let turns = turns::project(&model.entries);
        let scroller = cx.new(|cx| TranscriptState::new(turns.len(), cx));
        let model_picker = cx.new(|cx| ModelPicker::new(key.clone(), bridge.clone(), window, cx));
        let workspace_links =
            cx.new(|cx| WorkspaceLinks::new(key.clone(), bridge.clone(), window, cx));
        let subscriptions = vec![cx.subscribe_in(
            &composer,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(
                    event,
                    InputEvent::PressEnter {
                        secondary: true,
                        ..
                    }
                ) {
                    this.submit(window, cx);
                }
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        )];
        Self {
            key,
            bridge,
            model,
            composer,
            scroller,
            turns,
            clocks: HashMap::new(),
            markdown: HashMap::new(),
            expanded: HashSet::new(),
            revealed: HashSet::new(),
            submissions: Submissions::default(),
            attachments: Vec::new(),
            attachment_loading: false,
            queue_edit: None,
            error: None,
            model_info: serde_json::Value::Null,
            model_picker,
            workspace_links,
            config: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn apply_event(&mut self, event: &agent_ipc::UiEvent, cx: &mut Context<Self>) {
        let old_count = self.turns.len();
        self.model.apply_event(event);
        // Only boundaries are projected; Entry ids, draft indices and child
        // identities stay untouched in ChatModel.
        turns::update(
            &mut self.turns,
            &self.model.entries,
            self.model.changed_from,
        );
        let count = self.turns.len();
        if matches!(event, agent_ipc::UiEvent::UserPrompt(_))
            && let Some(turn) = self.turns.last()
        {
            self.clocks.entry(turn.id).or_insert((Instant::now(), None));
        }
        if !self.model.running
            && let Some((started, elapsed)) = self
                .turns
                .last()
                .and_then(|turn| self.clocks.get_mut(&turn.id))
        {
            elapsed.get_or_insert_with(|| started.elapsed());
        }
        let changed_from = turns::changed_turn(&self.turns, self.model.changed_from);
        self.scroller.update(cx, |state, cx| {
            if count > old_count {
                state.append(count - old_count, cx);
            } else if count < old_count {
                state.splice(count..old_count, 0, cx);
            }
            if changed_from < count {
                state.remeasure_items(changed_from..count, cx);
            }
        });
        cx.notify();
    }

    pub fn replace_history(
        &mut self,
        history: &[agent_llm::types::ChatMessage],
        cx: &mut Context<Self>,
    ) {
        self.model = ChatModel::from_history(history);
        self.turns = turns::project(&self.model.entries);
        self.clocks.clear();
        self.markdown.clear();
        self.expanded.clear();
        self.revealed.clear();
        self.scroller
            .update(cx, |state, cx| state.reset(self.turns.len(), cx));
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn receive_workspace_links(
        &mut self,
        request: u64,
        key: &SessionKey,
        data: &serde_json::Value,
        cx: &mut Context<Self>,
    ) -> bool {
        self.workspace_links
            .update(cx, |links, cx| links.receive(request, key, data, cx))
    }

    pub fn set_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.error = Some(message);
        cx.notify();
    }

    pub fn acknowledge_submission(
        &mut self,
        request_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.model_picker.update(cx, |picker, cx| {
            picker.complete(request_id, None, cx);
        });
        self.submissions.acknowledge(request_id);
        if let Some(edit) = self
            .queue_edit
            .as_ref()
            .filter(|edit| edit.pending == Some(request_id))
        {
            let return_focus = Focusable::focus_handle(edit.input.read(cx), cx).is_focused(window);
            self.queue_edit = None;
            if return_focus {
                self.focus(window, cx);
            }
            cx.notify();
        }
    }

    pub fn submission_failed(
        &mut self,
        request_id: u64,
        message: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.workspace_links.update(cx, |links, cx| {
            links.request_failed(request_id, message, cx)
        }) {
            return true;
        }
        if self.model_picker.update(cx, |picker, cx| {
            picker.complete(request_id, Some(message), cx)
        }) {
            return true;
        }
        if let Some(edit) = self
            .queue_edit
            .as_mut()
            .filter(|edit| edit.pending == Some(request_id))
        {
            edit.pending = None;
            edit.error = Some(format!("保存失败，编辑内容已保留：{message}"));
            cx.notify();
            return true;
        }
        let current = Draft {
            text: self.composer.read(cx).value().to_string(),
            attachments: self.attachments.clone(),
        };
        let Some(restored) = self.submissions.restore(request_id, current) else {
            return false;
        };
        self.composer
            .update(cx, |input, cx| input.set_value(restored.text, window, cx));
        self.attachments = restored.attachments;
        self.error = Some(format!("发送失败，已保留输入：{message}"));
        cx.notify();
        true
    }

    pub fn set_context(
        &mut self,
        model_info: &serde_json::Value,
        config: &ConfigInfo,
        cx: &mut Context<Self>,
    ) {
        self.model_picker
            .update(cx, |picker, cx| picker.set_context(model_info, config, cx));
        self.model_info = model_info.clone();
        self.config = Some(config.clone());
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.queue_edit.is_some() {
            return;
        }
        let text = self.composer.read(cx).value().to_string();
        let question_id = self
            .model
            .questions
            .first()
            .map(|question| question.request_id);
        if !can_submit(
            &text,
            !self.attachments.is_empty(),
            question_id.is_some(),
            self.attachment_loading,
        ) {
            return;
        }
        let command = if let Some(request_id) = question_id {
            Command::AnswerQuestion {
                key: self.key.clone(),
                request_id,
                answer: text.clone(),
            }
        } else {
            Command::Send {
                key: self.key.clone(),
                text: if self.attachments.is_empty() {
                    text.clone()
                } else {
                    build_payload(&text, &self.attachments)
                },
            }
        };
        match self.bridge.send(command) {
            Ok(request_id) => {
                let attachments = if question_id.is_none() {
                    std::mem::take(&mut self.attachments)
                } else {
                    Vec::new()
                };
                self.submissions
                    .insert(request_id, Draft { text, attachments });
                self.error = None;
                self.composer
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.scroller
                    .update(cx, |state, cx| state.scroll_to_end(cx));
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn choose_attachments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.attachment_loading || !self.model.questions.is_empty() {
            return;
        }
        self.attachment_loading = true;
        self.error = None;
        cx.notify();

        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("选择附件".into()),
        });
        let executor = cx.background_executor().clone();
        let workspace_root = std::path::PathBuf::from(&self.key.root);
        cx.spawn_in(window, async move |this, cx| {
            let result = match prompt.await {
                Ok(Ok(Some(paths))) => Some(
                    executor
                        .spawn(async move {
                            let mut loaded = Vec::new();
                            let mut errors = Vec::new();
                            for path in paths {
                                match Attachment::load(path, &workspace_root) {
                                    Ok(attachment) => loaded.push(attachment),
                                    Err(error) => errors.push(error),
                                }
                            }
                            (loaded, errors)
                        })
                        .await,
                ),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => Some((Vec::new(), vec![format!("选择附件失败：{error}")])),
                Err(_) => Some((Vec::new(), vec!["选择附件失败".into()])),
            };
            let _ = this.update_in(cx, |this, _, cx| {
                this.attachment_loading = false;
                if let Some((loaded, errors)) = result {
                    merge_attachments(&mut this.attachments, loaded);
                    if !errors.is_empty() {
                        this.error = Some(errors.join("；"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn command(&mut self, command: Command, cx: &mut Context<Self>) {
        if let Err(error) = self.bridge.send(command) {
            self.error = Some(error);
            cx.notify();
        }
    }

    fn edit_queued(
        &mut self,
        index: usize,
        expected: Arc<Vec<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Do not silently replace an unsaved edit or edit a stale rendered row.
        if self.queue_edit.is_some() || *expected != self.model.queued_prompts {
            return;
        }
        let Some(payload) = expected
            .get(index)
            .map(|payload| EditablePayload::parse(payload))
        else {
            return;
        };
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .default_value(payload.text.clone())
                .auto_grow(2, 5)
                .submit_on_enter(false)
        });
        let subscription = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
            if matches!(
                event,
                InputEvent::PressEnter {
                    secondary: true,
                    ..
                }
            ) {
                this.save_queued_edit(cx);
            }
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        self.queue_edit = Some(QueueEdit {
            index,
            expected,
            payload,
            input,
            pending: None,
            error: None,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn save_queued_edit(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.queue_edit.as_mut() else {
            return;
        };
        if edit.pending.is_some() || *edit.expected != self.model.queued_prompts {
            return;
        }
        let text = edit.input.read(cx).value().to_string();
        if text.trim().is_empty() && edit.payload.attachments.is_empty() {
            return;
        }
        match self.bridge.send(Command::EditQueued {
            key: self.key.clone(),
            index: edit.index,
            expected: edit.expected.as_ref().clone(),
            text: edit.payload.with_text(&text),
        }) {
            Ok(request_id) => {
                edit.pending = Some(request_id);
                edit.error = None;
            }
            Err(error) => edit.error = Some(format!("保存失败，编辑内容已保留：{error}")),
        }
        cx.notify();
    }

    fn render_queued(
        &self,
        index: usize,
        expected: Arc<Vec<String>>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(payload) = expected.get(index) else {
            return div().into_any_element();
        };
        let key = self.key.clone();
        let editing = self.queue_edit.is_some();
        let move_up = expected.clone();
        let move_down = expected.clone();
        let edit = expected.clone();
        h_flex()
            .h(px(36.))
            .w_full()
            .gap_2()
            .px_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(queued_label(payload)),
            )
            .child(
                Button::new(("edit-queued", index))
                    .ghost()
                    .small()
                    .label("编辑")
                    .accessibility_label(format!("编辑第 {} 条待发送消息", index + 1))
                    .disabled(editing)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_queued(index, edit.clone(), window, cx);
                    })),
            )
            .child(
                Button::new(("move-queued-up", index))
                    .ghost()
                    .small()
                    .label("上移")
                    .accessibility_label(format!("上移第 {} 条待发送消息", index + 1))
                    .disabled(editing || index == 0)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if index > 0 {
                            this.command(
                                Command::MoveQueued {
                                    key: this.key.clone(),
                                    from: index,
                                    to: index - 1,
                                    expected: move_up.as_ref().clone(),
                                },
                                cx,
                            );
                        }
                    })),
            )
            .child(
                Button::new(("move-queued-down", index))
                    .ghost()
                    .small()
                    .label("下移")
                    .accessibility_label(format!("下移第 {} 条待发送消息", index + 1))
                    .disabled(editing || index + 1 >= expected.len())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if index + 1 < move_down.len() {
                            this.command(
                                Command::MoveQueued {
                                    key: this.key.clone(),
                                    from: index,
                                    to: index + 1,
                                    expected: move_down.as_ref().clone(),
                                },
                                cx,
                            );
                        }
                    })),
            )
            .child(
                Button::new(("remove-queued", index))
                    .ghost()
                    .small()
                    .icon(Keyline::Close.icon())
                    .tooltip("移除待发送消息")
                    .disabled(editing)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.command(
                            Command::RemoveQueued {
                                key: key.clone(),
                                index,
                                expected: expected.as_ref().clone(),
                            },
                            cx,
                        );
                    })),
            )
            .into_any_element()
    }

    fn render_queued_editor(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let edit = self.queue_edit.as_ref()?;
        let theme = cx.theme().clone();
        let pending = edit.pending.is_some();
        let current = *edit.expected == self.model.queued_prompts;
        let empty =
            edit.input.read(cx).value().trim().is_empty() && edit.payload.attachments.is_empty();
        Some(
            v_flex()
                .id("queued-editor")
                .gap_2()
                .p_3()
                .border_1()
                .border_color(theme.border)
                .rounded_md()
                .child(
                    div()
                        .text_sm()
                        .child(format!("编辑第 {} 条待发送消息", edit.index + 1)),
                )
                .when(!edit.payload.attachments.is_empty(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("{} 个附件保持不变", edit.payload.attachments.len())),
                    )
                })
                .child(
                    Textarea::new(&edit.input)
                        .aria_label("待发送消息正文")
                        .disabled(pending),
                )
                .when(!current && !pending, |this| {
                    this.child(
                        div().text_sm().text_color(theme.danger).child(
                            "队列已变化，本次编辑未保存。可复制正文后取消，再选择待发送消息。",
                        ),
                    )
                })
                .when_some(edit.error.clone(), |this, error| {
                    this.child(div().text_sm().text_color(theme.danger).child(error))
                })
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("copy-queued-edit")
                                .ghost()
                                .small()
                                .label("复制正文")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(edit) = &this.queue_edit {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            edit.input.read(cx).value().to_string(),
                                        ));
                                    }
                                })),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("cancel-queued-edit")
                                .ghost()
                                .small()
                                .label("取消")
                                .disabled(pending)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.queue_edit = None;
                                    this.focus(window, cx);
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("save-queued-edit")
                                .primary()
                                .small()
                                .label(if pending { "保存中…" } else { "保存" })
                                .disabled(pending || !current || empty)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.save_queued_edit(cx);
                                })),
                        ),
                )
                .into_any_element(),
        )
    }

    fn markdown_view(&mut self, entry: &Entry, cx: &mut Context<Self>) -> AnyElement {
        let markdown = self.markdown.entry(entry.id).or_insert_with(|| Markdown {
            text: entry.text.clone(),
            state: cx.new(|cx| TextViewState::markdown(&entry.text, cx)),
        });
        if markdown.text != entry.text {
            markdown.state.update(cx, |state, cx| {
                if let Some(suffix) = entry.text.strip_prefix(&markdown.text) {
                    state.push_str(suffix, cx);
                } else {
                    state.set_text(&entry.text, cx);
                }
            });
            markdown.text = entry.text.clone();
        }
        let animate = self.model.running && !entry.complete && entry.kind == EntryKind::Assistant;
        let motion = if animate {
            gpui_kit::component::text::TextViewMotion::default()
                .with_stream_fade(Duration::from_millis(200))
                .with_stream_fade_stagger(Duration::from_millis(10))
                .with_stream_fade_easing(gpui_kit::base::motion::Easing::EaseOut)
        } else {
            gpui_kit::component::text::TextViewMotion::default()
        };
        TextView::new(&markdown.state)
            .font_family(cx.theme().font_family.clone())
            .text_size(px(14.))
            .line_height(relative(1.625))
            .style(
                gpui_kit::component::text::TextViewStyle::default()
                    .paragraph_gap(rems(0.5))
                    .heading_font_size(|level, _| {
                        px(match level {
                            1 => 20.,
                            2 => 18.,
                            3 => 16.,
                            _ => 14.,
                        })
                    }),
            )
            .selectable(true)
            .motion(motion)
            .on_link_click(|url, _, window, cx| {
                if !url.starts_with("https://") && !url.starts_with("http://") {
                    return;
                }
                let url = url.clone();
                let answer = window.prompt(
                    PromptLevel::Info,
                    "在浏览器中打开此链接？",
                    Some(&url),
                    &["取消", "打开"],
                    cx,
                );
                cx.spawn(async move |cx| {
                    if answer.await == Ok(1) {
                        cx.update(|cx| cx.open_url(&url));
                    }
                })
                .detach();
            })
            .into_any_element()
    }
}

impl Render for ChatPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let weak = cx.weak_entity();
        let mut approvals = Vec::new();
        for approval in self.model.approvals.clone() {
            let key = self.key.clone();
            let request = approval.request_id;
            let child = approval.child_id;
            let mut actions = h_flex().gap_2().child(
                Button::new(SharedString::from(format!("deny:{request}:{child:?}")))
                    .small()
                    .label("拒绝")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.command(
                            Command::Decide {
                                key: key.clone(),
                                request_id: request,
                                approved: false,
                                action: None,
                                child_id: child,
                            },
                            cx,
                        );
                    })),
            );
            let key = self.key.clone();
            actions = actions.when(approval.actions.is_empty(), |actions| {
                actions.child(
                    Button::new(SharedString::from(format!("approve:{request}:{child:?}")))
                        .primary()
                        .small()
                        .label("允许")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.command(
                                Command::Decide {
                                    key: key.clone(),
                                    request_id: request,
                                    approved: true,
                                    action: None,
                                    child_id: child,
                                },
                                cx,
                            );
                        })),
                )
            });
            for (index, action) in approval.actions.iter().enumerate() {
                let key = self.key.clone();
                let action_id = action.id.clone();
                actions = actions.child(
                    Button::new(SharedString::from(format!(
                        "approval:{request}:{child:?}:{index}"
                    )))
                    .small()
                    .label(action.label.clone())
                    .when_some(action.description.clone(), |this, description| {
                        this.tooltip(description)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.command(
                            Command::Decide {
                                key: key.clone(),
                                request_id: request,
                                approved: true,
                                action: Some(action_id.clone()),
                                child_id: child,
                            },
                            cx,
                        );
                    })),
                );
            }
            approvals.push(
                v_flex()
                    .gap_2()
                    .pb_2()
                    .text_xs()
                    .child(format!(
                        "{} · {}",
                        if approval.actions.is_empty() {
                            "允许执行"
                        } else {
                            "需要访问"
                        },
                        approval.tool_name
                    ))
                    .when(approval.fuzzy, |this| {
                        this.child("补丁使用了近似匹配，请核对修改内容。")
                    })
                    .child(
                        div()
                            .id(("approval-diff", request))
                            .max_h(px(180.))
                            .overflow_y_scroll()
                            .text_xs()
                            .child(approval.diff),
                    )
                    .child(actions)
                    .into_any_element(),
            );
        }
        for question in self.model.questions.clone() {
            let mut options = h_flex().gap_2().flex_wrap();
            for (index, option) in question.options.into_iter().enumerate() {
                let key = self.key.clone();
                let answer = option.label.clone();
                let request_id = question.request_id;
                options = options.child(
                    Button::new(SharedString::from(format!(
                        "answer:{}:{index}",
                        question.request_id
                    )))
                    .small()
                    .when_some(option.description.clone(), |this, description| {
                        this.tooltip(description)
                    })
                    .label(option.label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.command(
                            Command::AnswerQuestion {
                                key: key.clone(),
                                request_id,
                                answer: answer.clone(),
                            },
                            cx,
                        );
                    })),
                );
            }
            approvals.push(
                v_flex()
                    .gap_2()
                    .pb_2()
                    .text_xs()
                    .child(question.text)
                    .child(options)
                    .into_any_element(),
            );
        }
        let status = if self.model.status.is_empty() {
            "就绪"
        } else {
            &self.model.status
        };
        let mode = self
            .config
            .as_ref()
            .map(|config| config.agent_mode.as_str())
            .unwrap_or("build");
        let permission = self
            .config
            .as_ref()
            .map(|config| config.permission_mode.as_str())
            .unwrap_or("default");
        let agent_bridge = self.bridge.clone();
        let permission_bridge = self.bridge.clone();
        let agent_key = self.key.clone();
        let permission_key = self.key.clone();
        let selected_agent = mode.to_owned();
        let selected_permission = permission.to_owned();
        let toolbar = h_flex()
            .gap(px(6.))
            .pt(px(4.))
            .child(
                Button::new("attach-files")
                    .ghost()
                    .small()
                    .w(px(28.))
                    .h(px(28.))
                    .p_0()
                    .rounded_full()
                    .bg(if theme.is_dark() {
                        rgb(0x2b2b32)
                    } else {
                        rgb(0xf4f4f5)
                    })
                    .border_1()
                    .border_color(crate::ui::composer::alpha(
                        if theme.is_dark() { 0x34343d } else { 0xe4e4e7 },
                        0.8,
                    ))
                    .text_color(theme.muted_foreground)
                    .icon(Keyline::Plus.icon())
                    .tooltip(if self.attachment_loading {
                        "正在读取附件"
                    } else if !self.model.questions.is_empty() {
                        "回答问题时不能添加附件"
                    } else {
                        "添加文件或图片"
                    })
                    .disabled(self.attachment_loading || !self.model.questions.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.choose_attachments(window, cx);
                    })),
            )
            .child(
                Menu::new(
                    "agent-mode-menu-popup",
                    crate::ui::composer::pill(
                        "agent-mode-menu",
                        match mode {
                            "plan" => Keyline::Route,
                            "goal" => Keyline::Flag,
                            "office" => Keyline::File,
                            _ => Keyline::Hammer,
                        },
                        match mode {
                            "plan" => "计划",
                            "goal" => "目标",
                            "office" => "工作",
                            _ => "构建",
                        },
                        cx,
                    )
                    .accessibility_label("代理模式"),
                    move |mut menu, _, _| {
                        for (mode, label) in [("build", "构建"), ("plan", "计划"), ("goal", "目标")]
                        {
                            let bridge = agent_bridge.clone();
                            let key = agent_key.clone();
                            menu = menu.item(
                                PopupMenuItem::new(label)
                                    .checked(mode == selected_agent)
                                    .on_click(move |_, _, _| {
                                        let _ = bridge.send(Command::SetAgentMode {
                                            key: key.clone(),
                                            mode: mode.into(),
                                        });
                                    }),
                            );
                        }
                        menu
                    },
                )
                .anchor(Anchor::BottomLeft),
            )
            .child(
                Menu::new(
                    "permission-menu-popup",
                    crate::ui::composer::pill(
                        "permission-menu",
                        match permission {
                            "auto" => Keyline::Zap,
                            "acceptEdits" => Keyline::PenLine,
                            "dontAsk" => Keyline::Ban,
                            "bypassPermissions" => Keyline::ShieldX,
                            _ => Keyline::Shield,
                        },
                        permission_label(permission),
                        cx,
                    )
                    .accessibility_label("权限模式"),
                    move |mut menu, _, _| {
                        for mode in [
                            "default",
                            "acceptEdits",
                            "auto",
                            "dontAsk",
                            "bypassPermissions",
                        ] {
                            let bridge = permission_bridge.clone();
                            let key = permission_key.clone();
                            menu = menu.item(
                                PopupMenuItem::new(permission_label(mode))
                                    .checked(mode == selected_permission)
                                    .on_click(move |_, _, _| {
                                        let _ = bridge.send(Command::SetPermissionMode {
                                            key: key.clone(),
                                            mode: mode.into(),
                                        });
                                    }),
                            );
                        }
                        menu
                    },
                )
                .anchor(Anchor::BottomLeft),
            )
            .child(
                WorkspaceLinks::control(&self.workspace_links, cx).trigger(
                    crate::ui::composer::pill(
                        "workspace-links-trigger",
                        Keyline::Folders,
                        "关联工作区",
                        cx,
                    )
                    .accessibility_label("关联工作区"),
                ),
            )
            .child(div().flex_1())
            .child(ModelPicker::control(&self.model_picker, cx));
        let transcript = div().flex_1().min_h_0().w_full().when_else(
            self.model.entries.is_empty(),
            |this| {
                let (greeting, icon, tone) = crate::ui::greeting::time_greeting(theme.is_dark());
                this.flex().items_center().justify_center().child(
                    v_flex()
                        .items_center()
                        .gap_3()
                        .pb(px(64.))
                        .child(
                            h_flex()
                                .gap_3()
                                .items_center()
                                .child(icon.icon().size(px(24.)).text_color(rgb(tone)))
                                .child(
                                    div()
                                        .text_size(px(26.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(format!("{greeting}，今天想做点什么？")),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(theme.muted_foreground)
                                .child("在下方输入框发送消息，开始新对话"),
                        ),
                )
            },
            |this| {
                let rail = self.turn_rail(cx);
                this.relative()
                    .child(scroller::transcript(
                        SharedString::from(format!("transcript:{}:{}", self.key.root, self.key.id)),
                        self.scroller.clone(),
                        move |index, window, cx| {
                            weak.update(cx, |this, cx| this.render_turn(index, window, cx))
                                .unwrap_or_else(|_| div().into_any_element())
                        },
                        window,
                        cx,
                    ))
                    .child(
                        div()
                            .absolute()
                            .left(px(8.))
                            .top_0()
                            .bottom_0()
                            .flex()
                            .items_center()
                            .child(rail),
                    )
            },
        );
        let toolbar = toolbar
            .when(self.model.running, |this| {
                this.child(
                    Button::new("stop")
                        .small()
                        .icon(Keyline::Stop.icon())
                        .label("停止")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.command(
                                Command::Cancel {
                                    key: this.key.clone(),
                                },
                                cx,
                            );
                        })),
                )
            })
            .child(
                Button::new("send")
                    .primary()
                    .small()
                    .w(px(32.))
                    .h(px(32.))
                    .rounded_full()
                    .ml(px(2.))
                    .icon(Keyline::Send.icon())
                    .tooltip(if self.model.questions.is_empty() {
                        "发送消息"
                    } else {
                        "回答问题"
                    })
                    .disabled(
                        self.queue_edit.is_some()
                            || !can_submit(
                                self.composer.read(cx).value().as_ref(),
                                !self.attachments.is_empty(),
                                !self.model.questions.is_empty(),
                                self.attachment_loading,
                            ),
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
            );
        let attachment_chips: Vec<_> = self
            .attachments
            .iter()
            .enumerate()
            .map(|(index, attachment)| {
                let label = format!(
                    "{} · {}{}",
                    if attachment.is_image {
                        "图片"
                    } else {
                        "文件"
                    },
                    attachment.name,
                    if attachment.truncated {
                        "（已截断）"
                    } else {
                        ""
                    }
                );
                h_flex()
                    .max_w(px(280.))
                    .gap_1()
                    .pl_2()
                    .rounded_md()
                    .bg(theme.muted)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(label),
                    )
                    .child(
                        Button::new(("remove-attachment", index))
                            .ghost()
                            .small()
                            .icon(Keyline::Close.icon())
                            .tooltip("移除附件")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if index < this.attachments.len() {
                                    this.attachments.remove(index);
                                    cx.notify();
                                }
                            })),
                    )
                    .into_any_element()
            })
            .collect();
        let composer = v_flex()
            .w_full()
            .max_w(px(768.))
            .mx_auto()
            .p(px(12.))
            .gap(px(8.))
            .border_1()
            .border_color(theme.border)
            .rounded(px(layout::COMPOSER_RADIUS))
            .bg(if theme.is_dark() {
                rgb(0x232329)
            } else {
                rgb(0xffffff)
            })
            .shadow(vec![BoxShadow {
                color: crate::ui::composer::alpha(0x000000, 0.025),
                offset: point(px(0.), px(2.)),
                blur_radius: px(12.),
                spread_radius: px(0.),
                inset: false,
            }])
            .children(approvals)
            .when(!attachment_chips.is_empty(), |this| {
                this.child(h_flex().gap(px(4.)).flex_wrap().children(attachment_chips))
            })
            .when(self.attachment_loading, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("正在读取附件…"),
                )
            })
            .when(
                !self.model.questions.is_empty() && !self.attachments.is_empty(),
                |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("附件会在回答问题后随下一条消息发送"),
                    )
                },
            )
            .child(
                Textarea::new(&self.composer)
                    .h_auto()
                    .min_h(px(42.))
                    .max_h(px(180.))
                    .text_size(px(14.))
                    .line_height(px(22.75))
                    .appearance(false)
                    .bordered(false)
                    .aria_label("消息"),
            )
            .child(toolbar);
        let queue_count = self.model.queued_prompts.len();
        let queued = Arc::new(self.model.queued_prompts.clone());
        let queue_list = (queue_count > 0 && self.queue_edit.is_none()).then(|| {
            uniform_list(
                "queued-prompts",
                queue_count,
                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                    range
                        .map(|index| this.render_queued(index, queued.clone(), cx))
                        .collect::<Vec<_>>()
                }),
            )
            .h(px(queue_count.min(4) as f32 * 36.))
            .into_any_element()
        });
        let queued_editor = self.render_queued_editor(cx);
        let footer = v_flex()
            .w_full()
            .pl(px(44.))
            .pr(px(16.))
            .pt(px(12.))
            .pb(px(24.))
            .gap(px(8.))
            .when_some(queue_list, |this, queue_list| {
                this.child(
                    v_flex()
                        .gap(px(4.))
                        .border_1()
                        .border_color(theme.border)
                        .rounded_md()
                        .py(px(4.))
                        .child(
                            h_flex()
                                .px(px(8.))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(format!("待发送 · {queue_count}")),
                                )
                                .child(
                                    Button::new("resume-queued")
                                        .ghost()
                                        .small()
                                        .label("继续")
                                        .disabled(self.model.running || self.queue_edit.is_some())
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.command(
                                                Command::ResumeQueued {
                                                    key: this.key.clone(),
                                                },
                                                cx,
                                            );
                                        })),
                                ),
                        )
                        .child(queue_list),
                )
            })
            .when_some(queued_editor, |this, editor| this.child(editor))
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(theme.danger).child(error))
            })
            .when(self.model.running, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(status.to_owned()),
                )
            })
            .when(self.queue_edit.is_none(), |this| this.child(composer));
        v_flex()
            .size_full()
            .min_h_0()
            .child(transcript)
            .child(footer)
    }
}

#[cfg(all(test, feature = "test-support"))]
#[path = "queued_tests.rs"]
mod queued_tests;

#[cfg(test)]
mod tests {
    use super::{
        Attachment, ChatModel, build_payload, can_submit, display_payload, fast_mode_state,
        queued_label,
    };
    use agent_ipc::UiEvent;
    use agent_llm::types::ChatMessage;
    use serde_json::json;

    #[test]
    fn fast_control_requires_explicit_model_capability() {
        let supported = json!({"supports_fast_mode": true});
        let unsupported = json!({"supports_fast_mode": false});
        assert_eq!(
            fast_mode_state(None, &json!({"active_fast_mode": true})),
            None
        );
        assert_eq!(
            fast_mode_state(Some(&unsupported), &json!({"active_fast_mode": true})),
            None
        );
        assert_eq!(fast_mode_state(Some(&supported), &json!({})), Some(false));
        assert_eq!(
            fast_mode_state(Some(&supported), &json!({"active_fast_mode": true})),
            Some(true)
        );
    }

    #[test]
    fn attachments_enable_prompts_but_not_empty_question_answers() {
        assert!(can_submit("", true, false, false));
        assert!(!can_submit("  ", true, true, false));
        assert!(can_submit("answer", true, true, false));
        assert!(!can_submit("message", true, false, true));
        assert!(!can_submit("", false, false, false));
    }

    #[test]
    fn queued_summary_hides_attachment_bodies_and_limits_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private.txt");
        std::fs::write(&path, "secret attachment body").unwrap();
        let attachment = Attachment::load(path, dir.path()).unwrap();
        let payload = build_payload("first\nsecond", &[attachment]);
        assert_eq!(queued_label(&payload), "first second · 1 个附件");
        assert!(!queued_label(&payload).contains("secret"));
        assert_eq!(
            queued_label(&"字".repeat(121)),
            format!("{}…", "字".repeat(120))
        );
        assert_eq!(queued_label("hello\u{202e}world"), "helloworld");
    }

    #[test]
    fn attachment_messages_display_identically_after_history_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, "file content not shown in the bubble").unwrap();
        let attachment = Attachment::load(path, dir.path()).unwrap();
        let payload = build_payload("Please review", &[attachment]);

        let mut streamed = ChatModel::default();
        streamed.apply_event(&UiEvent::UserPrompt(payload.clone()));
        let restored = ChatModel::from_history(&[ChatMessage::user(payload)]);
        let streamed_display = display_payload(&streamed.entries[0].text);
        let restored_display = display_payload(&restored.entries[0].text);

        assert_eq!(streamed_display.0, "Please review");
        assert_eq!(streamed_display.1.len(), 1);
        assert_eq!(streamed_display.1[0].name, "notes.txt");
        assert_eq!(restored_display.0, streamed_display.0);
        assert_eq!(restored_display.1[0].name, streamed_display.1[0].name);
        assert!(!restored_display.0.contains("file content"));
    }

    #[cfg(feature = "test-support")]
    #[gpui_kit::test]
    fn file_picker_updates_and_removes_composer_attachments(cx: &mut gpui_kit::TestAppContext) {
        use super::ChatPanel;
        use crate::bridge::{BridgeEvent, HostBridge, SessionKey};
        use gpui_kit::{AppContext, WindowOptions, test::TestWindowExt};
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let path = workspace.join("notes.txt");
        std::fs::write(&path, "test attachment").unwrap();
        let new_path = workspace.join("new.txt");
        std::fs::write(&new_path, "new attachment").unwrap();
        let paths =
            agent_paths::AppPaths::new(dir.path().join("config"), dir.path().join("data"), ".husk");
        let bridge = Arc::new(HostBridge::spawn(paths).unwrap());
        let events = bridge.events();
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
            window.render_frame(cx);
            assert_eq!(window.find("send").disabled(), Some(true));
            window.click("attach-files", cx);
        })
        .unwrap();
        assert!(cx.did_prompt_for_paths());
        cx.simulate_path_prompt_response(|options| {
            assert!(options.files && options.multiple && !options.directories);
            Some(vec![path])
        });
        cx.run_until_parked();
        cx.update(|cx| assert_eq!(panel.read(cx).attachments.len(), 1));
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(!window.find("send").disabled().unwrap_or(false));
            panel.update(cx, |panel, cx| {
                panel
                    .composer
                    .update(cx, |input, cx| input.set_value("original", window, cx));
            });
            window.render_frame(cx);
            window.click("send", cx);
            panel.update(cx, |panel, cx| {
                assert!(panel.attachments.is_empty());
                panel
                    .composer
                    .update(cx, |input, cx| input.set_value("new draft", window, cx));
            });
            window.render_frame(cx);
            window.click("attach-files", cx);
        })
        .unwrap();
        cx.simulate_path_prompt_response(|_| Some(vec![new_path]));
        cx.run_until_parked();
        let deadline = Instant::now() + Duration::from_secs(5);
        let (request_id, message) = loop {
            match events.try_recv() {
                Ok(BridgeEvent::Error {
                    request_id,
                    message,
                }) if request_id != 0 => break (request_id, message),
                Ok(_) | Err(async_channel::TryRecvError::Empty) => {
                    assert!(Instant::now() < deadline, "submission failure timed out");
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("bridge stopped: {error}"),
            }
        };
        cx.update_window(window.into(), |_, window, cx| {
            panel.update(cx, |panel, cx| {
                assert!(panel.submission_failed(request_id, &message, window, cx));
                assert_eq!(panel.attachments.len(), 2);
                assert_eq!(panel.composer.read(cx).value(), "original\n\nnew draft");
            });
            window.render_frame(cx);
            window.click(("remove-attachment", 0usize), cx);
            window.click(("remove-attachment", 0usize), cx);
            panel.update(cx, |panel, cx| {
                panel
                    .composer
                    .update(cx, |input, cx| input.set_value("", window, cx));
            });
            window.render_frame(cx);
            assert_eq!(window.find("send").disabled(), Some(true));
        })
        .unwrap();
        cx.update(|cx| assert!(panel.read(cx).attachments.is_empty()));
        cx.update_window(window.into(), |_, window, cx| {
            panel.update(cx, |panel, cx| {
                panel.apply_event(
                    &UiEvent::QueuedPrompts {
                        items: vec!["queued task".into()],
                    },
                    cx,
                );
            });
            window.render_frame(cx);
            assert!(!window.find("resume-queued").disabled().unwrap_or(false));
            assert!(window.find(("remove-queued", 0usize)).visible());
            panel.update(cx, |panel, cx| {
                panel.apply_event(&UiEvent::StateChanged(agent_ipc::AgentState::Reasoning), cx);
            });
            window.render_frame(cx);
            assert_eq!(window.find("resume-queued").disabled(), Some(true));
            window.click(("remove-queued", 0usize), cx);
        })
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match events.try_recv() {
                Ok(BridgeEvent::Error { request_id, .. }) if request_id != 0 => break,
                Ok(_) | Err(async_channel::TryRecvError::Empty) => {
                    assert!(Instant::now() < deadline, "queue rejection timed out");
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("bridge stopped: {error}"),
            }
        }
        cx.update_window(window.into(), |_, window, cx| {
            panel.update(cx, |panel, cx| {
                assert_eq!(panel.model.queued_prompts, ["queued task"]);
                panel.apply_event(&UiEvent::QueuedPrompts { items: vec![] }, cx);
            });
            window.render_frame(cx);
            assert!(window.try_find(("remove-queued", 0usize)).is_none());
        })
        .unwrap();
        bridge.shutdown();
    }
}
