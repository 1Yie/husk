//! Full-window settings navigation. The shell owns the transition back to work.
mod editor;
mod general;
mod pages;
use crate::bridge::{Command, HostBridge, SettingsCommand, Snapshot};
use editor::{Editor, EditorAction, FieldSpec};
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::{collections::HashSet, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsEvent {
    Back,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsPage {
    General,
    Appearance,
    Terminal,
    Instructions,
    Rules,
    Model,
    Memory,
    Skills,
    Plugins,
    Mcp,
    Subagent,
    Stats,
    Developer,
    About,
}
impl SettingsPage {
    fn title(self) -> &'static str {
        match self {
            Self::General => "常规",
            Self::Appearance => "外观",
            Self::Terminal => "终端",
            Self::Instructions => "指令",
            Self::Rules => "规则",
            Self::Model => "模型",
            Self::Memory => "记忆",
            Self::Skills => "技能",
            Self::Plugins => "插件",
            Self::Mcp => "MCP",
            Self::Subagent => "子代理",
            Self::Stats => "统计",
            Self::Developer => "开发者选项",
            Self::About => "关于",
        }
    }
    fn icon(self) -> crate::ui::icons::Keyline {
        use crate::ui::icons::Keyline;
        match self {
            Self::General => Keyline::Sliders,
            Self::Appearance => Keyline::Palette,
            Self::Terminal => Keyline::Terminal,
            Self::Instructions => Keyline::File,
            Self::Rules => Keyline::ListCheck,
            Self::Model => Keyline::Cpu,
            Self::Memory => Keyline::Brain,
            Self::Skills => Keyline::Sparkles,
            Self::Plugins => Keyline::Plug,
            Self::Mcp => Keyline::Wrench,
            Self::Subagent => Keyline::Bot,
            Self::Stats => Keyline::Chart,
            Self::Developer => Keyline::Flask,
            Self::About => Keyline::Info,
        }
    }
}

pub struct SettingsPanel {
    bridge: Arc<HostBridge>,
    snapshot: Option<Snapshot>,
    error: Option<String>,
    pending: HashSet<u64>,
    page: SettingsPage,
    editor: Option<Entity<Editor>>,
    editor_request: Option<u64>,
    close_editor: bool,
}
impl EventEmitter<SettingsEvent> for SettingsPanel {}
impl SettingsPanel {
    pub fn new(bridge: Arc<HostBridge>) -> Self {
        Self {
            bridge,
            snapshot: None,
            error: None,
            pending: HashSet::new(),
            page: SettingsPage::General,
            editor: None,
            editor_request: None,
            close_editor: false,
        }
    }
    pub fn set_snapshot(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.snapshot = Some(snapshot);
        cx.notify();
    }
    pub fn set_page(&mut self, page: SettingsPage, cx: &mut Context<Self>) {
        self.page = page;
        cx.notify();
    }
    /// Refresh on settings takeover, rather than doing settings I/O on chat events.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.settings(SettingsCommand::Refresh, cx);
    }
    pub fn acknowledge(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.pending.remove(&id) {
            self.error = None;
            if self.editor_request == Some(id) {
                self.close_editor = true;
                self.editor_request = None;
            }
            cx.notify();
        }
    }
    pub fn request_failed(&mut self, id: u64, message: &str, cx: &mut Context<Self>) -> bool {
        if !self.pending.remove(&id) {
            return false;
        }
        self.error = Some(message.to_owned());
        if self.editor_request == Some(id) {
            self.editor_request = None;
            if let Some(editor) = &self.editor {
                editor.update(cx, |e, cx| {
                    e.pending = false;
                    e.error = Some(message.to_owned());
                    cx.notify();
                });
            }
        }
        cx.notify();
        true
    }
    fn send(&mut self, command: Command, cx: &mut Context<Self>) -> Option<u64> {
        if !self.pending.is_empty() {
            return None;
        }
        self.error = None;
        let id = match self.bridge.send(command) {
            Ok(id) => {
                self.pending.insert(id);
                Some(id)
            }
            Err(error) => {
                self.error = Some(error);
                None
            }
        };
        cx.notify();
        id
    }
    fn settings(&mut self, command: SettingsCommand, cx: &mut Context<Self>) {
        self.send(Command::Settings(command), cx);
    }
    fn edit(
        &mut self,
        title: &str,
        action: EditorAction,
        fields: Vec<FieldSpec>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.pending.is_empty() {
            return;
        }
        let editor = cx.new(|cx| Editor::new(action, fields, window, cx));
        self.editor = Some(editor.clone());
        self.error = None;
        let panel = cx.weak_entity();
        let title = title.to_owned();
        window.open_alert_dialog(cx, move |dialog, _, cx| {
            let submit = panel.clone();
            let cancel = panel.clone();
            let draft = editor.clone();
            let busy = editor.read(cx).pending;
            dialog
                .confirm()
                .entrance(gpui_kit::component::dialog::DialogEntrance::FadeSlide)
                .width(px(620.))
                .title(title.clone())
                .description("更改将写入真实宿主配置。保存失败时保留草稿。")
                .ok_text(if busy { "保存中…" } else { "保存" })
                .cancel_text("取消")
                .child(editor.clone())
                .on_ok(move |_, _, cx| {
                    if draft.read(cx).pending {
                        return false;
                    }
                    match draft.read(cx).command(cx) {
                        Ok(command) => {
                            let draft = draft.clone();
                            let _ = submit.update(cx, |this, cx| {
                                if let Some(id) = this.send(Command::Settings(command), cx) {
                                    this.editor_request = Some(id);
                                    draft.update(cx, |e, cx| {
                                        e.pending = true;
                                        e.error = None;
                                        cx.notify();
                                    });
                                } else if let Some(error) = &this.error {
                                    draft.update(cx, |e, cx| {
                                        e.error = Some(error.clone());
                                        cx.notify();
                                    });
                                }
                            });
                        }
                        Err(error) => draft.update(cx, |e, cx| {
                            e.error = Some(error);
                            cx.notify();
                        }),
                    }
                    false
                })
                .on_cancel(move |_, _, cx| {
                    cancel
                        .update(cx, |this, _| {
                            if this.editor_request.is_some() {
                                false
                            } else {
                                this.editor = None;
                                true
                            }
                        })
                        .unwrap_or(true)
                })
        });
        cx.notify();
    }
    fn confirm(
        &mut self,
        title: String,
        command: SettingsCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel = cx.weak_entity();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let panel = panel.clone();
            let command = command.clone();
            dialog
                .confirm()
                .entrance(gpui_kit::component::dialog::DialogEntrance::FadeSlide)
                .title(title.clone())
                .description("此操作会修改实际存储，无法撤销。失败会显示错误，不会假报成功。")
                .ok_text("确认")
                .cancel_text("取消")
                .on_ok(move |_, _, cx| {
                    panel
                        .update(cx, |this, cx| {
                            this.send(Command::Settings(command.clone()), cx).is_some()
                        })
                        .unwrap_or(false)
                })
        });
    }
}
impl Render for SettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.close_editor {
            self.close_editor = false;
            self.editor = None;
            window.close_dialog(cx);
        }
        let theme = cx.theme().clone();
        let mut nav = v_flex().p(px(12.)).gap(px(16.)).child(
            Button::new("settings-back")
                .ghost()
                .w_full()
                .h(px(32.))
                .px(px(8.))
                .rounded(px(14.))
                .justify_start()
                .text_size(px(13.))
                .line_height(px(20.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.muted_foreground)
                .child(
                    h_flex()
                        .w_full()
                        .gap(px(8.))
                        .child(crate::ui::icons::Keyline::Back.icon().size(px(16.)))
                        .child("返回工作区"),
                )
                .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Back))),
        );
        for (group, pages) in [
            (
                "基础设置",
                vec![
                    SettingsPage::General,
                    SettingsPage::Appearance,
                    SettingsPage::Terminal,
                ],
            ),
            (
                "智能体",
                vec![
                    SettingsPage::Instructions,
                    SettingsPage::Rules,
                    SettingsPage::Model,
                    SettingsPage::Memory,
                    SettingsPage::Skills,
                    SettingsPage::Plugins,
                    SettingsPage::Mcp,
                    SettingsPage::Subagent,
                ],
            ),
            (
                "系统",
                vec![
                    SettingsPage::Stats,
                    SettingsPage::Developer,
                    SettingsPage::About,
                ],
            ),
        ] {
            let mut section = v_flex().gap(px(4.)).child(
                div()
                    .px(px(12.))
                    .pb(px(4.))
                    .text_size(px(11.))
                    .line_height(px(20.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.muted_foreground)
                    .child(group),
            );
            for page in pages {
                if page == SettingsPage::Developer
                    && !self.snapshot.as_ref().is_some_and(|snapshot| {
                        snapshot.model_info["settings"]["developer_ui"].as_bool() == Some(true)
                    })
                {
                    continue;
                }
                section = section.child(
                    Button::new(("settings-nav", page as usize))
                        .ghost()
                        .w_full()
                        .h(px(36.))
                        .px(px(12.))
                        .rounded(px(14.))
                        .justify_start()
                        .text_size(px(13.))
                        .line_height(px(20.))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(rgb(if theme.is_dark() {
                            if self.page == page {
                                0xf4f4f5
                            } else {
                                0xa1a1aa
                            }
                        } else if self.page == page {
                            0x18181b
                        } else {
                            0x52525b
                        }))
                        .when(self.page == page, |button| {
                            button.bg(crate::ui::composer::alpha(
                                if theme.is_dark() { 0xfafafa } else { 0x09090b },
                                0.06,
                            ))
                        })
                        .child(
                            h_flex()
                                .w_full()
                                .gap(px(10.))
                                .child(page.icon().icon().size(px(16.)))
                                .child(page.title()),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.set_page(page, cx))),
                );
            }
            nav = nav.child(section);
        }
        let nav = v_flex()
            .w(px(240.))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .child(
                h_flex().h(px(36.)).flex_shrink_0().px(px(12.)).child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("设置"),
                ),
            )
            .child(nav);
        let mut content = v_flex()
            .w_full()
            .max_w(px(768.))
            .mx_auto()
            .gap(px(24.))
            .px(px(40.))
            .pt(px(32.))
            .pb(px(64.))
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(20.))
                            .line_height(px(28.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.page.title()),
                    )
                    .when(self.error.is_some(), |header| {
                        header.child(
                            Button::new("settings-refresh")
                                .small()
                                .ghost()
                                .label("刷新")
                                .disabled(!self.pending.is_empty())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.settings(SettingsCommand::Refresh, cx)
                                })),
                        )
                    }),
            );
        if let Some(error) = &self.error {
            content = content.child(
                div()
                    .p(px(12.))
                    .rounded_md()
                    .border_1()
                    .border_color(theme.danger)
                    .text_color(theme.danger)
                    .child(error.clone()),
            );
        }
        if let Some(snapshot) = self.snapshot.clone() {
            content = content.child(self.render_page(&snapshot, window, cx));
        } else {
            content = content.child("正在读取设置…");
        }
        h_flex()
            .id("settings-panel")
            .size_full()
            .bg(rgb(if theme.is_dark() { 0x232329 } else { 0xffffff }))
            .text_color(theme.foreground)
            .child(nav.id("settings-nav").overflow_y_scroll())
            .child(
                div()
                    .id("settings-content")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .child(content),
            )
    }
}
#[cfg(all(test, feature = "test-support"))]
mod tests;
