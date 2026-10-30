//! Session activity dock. All host work crosses the owner bridge; no UI-side
//! filesystem access and no fabricated rows.
#[cfg(test)]
mod tests;
mod view;

use std::{collections::HashMap, sync::Arc, time::Duration};

use gpui_kit::component::{
    input::InputState,
    text::{TextView, TextViewState},
};
use gpui_kit::*;
use serde_json::Value;

use crate::bridge::{ActivityCommand, Command, HostBridge, SessionKey};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActivityTab {
    #[default]
    Artifacts,
    Jobs,
    Subagents,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Detail {
    Artifact(String),
    Logs(String),
    Child(u64),
}

#[derive(Clone)]
struct Pending {
    key: SessionKey,
    slot: &'static str,
    detail: Option<Detail>,
}

pub struct ActivityPanel {
    bridge: Arc<HostBridge>,
    session: Option<SessionKey>,
    tab: ActivityTab,
    visible: bool,
    header_visible: bool,
    artifacts: Vec<Value>,
    jobs: Vec<Value>,
    children: Vec<Value>,
    loading: bool,
    loaded: bool,
    error: Option<String>,
    artifact_error: Option<String>,
    child_error: Option<String>,
    pending: HashMap<u64, Pending>,
    latest: HashMap<&'static str, u64>,
    detail: Option<Detail>,
    detail_loading: bool,
    detail_error: Option<String>,
    detail_text: String,
    detail_view: Option<Entity<TextViewState>>,
    steer: Entity<InputState>,
    notice: Option<String>,
    scroll: ScrollHandle,
    poll: Option<Task<()>>,
}

impl ActivityPanel {
    pub fn new(bridge: Arc<HostBridge>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            bridge,
            session: None,
            tab: ActivityTab::Artifacts,
            visible: false,
            header_visible: true,
            artifacts: Vec::new(),
            jobs: Vec::new(),
            children: Vec::new(),
            loading: false,
            loaded: false,
            error: None,
            artifact_error: None,
            child_error: None,
            pending: HashMap::new(),
            latest: HashMap::new(),
            detail: None,
            detail_loading: false,
            detail_error: None,
            detail_text: String::new(),
            detail_view: None,
            steer: cx
                .new(|cx| InputState::new(window, cx).placeholder("向运行中的子代理发送指导…")),
            notice: None,
            scroll: ScrollHandle::new(),
            poll: None,
        }
    }

    pub fn set_session(
        &mut self,
        key: Option<SessionKey>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.session == key {
            return;
        }
        self.poll = None;
        self.session = key;
        self.artifacts.clear();
        self.jobs.clear();
        self.children.clear();
        // Keep old request identities until their replies arrive, so a late
        // failure is consumed here rather than surfaced as a global error
        // against the newly selected session.
        self.latest.clear();
        self.loaded = false;
        self.loading = false;
        self.error = None;
        self.artifact_error = None;
        self.child_error = None;
        self.close_detail(cx);
        self.notice = None;
        self.scroll.set_offset(point(px(0.), px(0.)));
        if self.visible {
            self.refresh(window, cx);
            self.start_poll(cx);
        }
        cx.notify();
    }

    pub fn set_header_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.header_visible = visible;
        cx.notify();
    }

    pub fn count(&self, tab: ActivityTab) -> usize {
        match tab {
            ActivityTab::Artifacts => self.artifacts.len(),
            ActivityTab::Jobs => self.jobs.len(),
            ActivityTab::Subagents => self.children.len(),
        }
    }

    /// The shell owns dock / settings visibility. Hidden panels retain data
    /// and selected details, but perform no polling or session-open reads.
    pub fn set_visible(&mut self, visible: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        self.poll = None;
        if visible {
            self.refresh(window, cx);
            self.start_poll(cx);
        }
        cx.notify();
    }

    fn start_poll(&mut self, cx: &mut Context<Self>) {
        if self.visible && self.session.is_some() && self.poll.is_none() {
            self.poll = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(5)).await;
                    if this
                        .update(cx, |this, cx| {
                            // Pending reads / destructive operations are never
                            // overtaken by the automatic reconciliation.
                            if this.visible && this.pending.is_empty() {
                                this.refresh_inner(cx);
                                if let Some(Detail::Logs(job)) = this.detail.clone() {
                                    if this.jobs.iter().any(|row| {
                                        row["id"].as_str() == Some(&job)
                                            && matches!(
                                                field(&row["state"], "kind"),
                                                "running" | "paused"
                                            )
                                    }) {
                                        this.load_detail(Detail::Logs(job), false, cx);
                                    }
                                } else if let Some(Detail::Child(child_id)) = this.detail.clone() {
                                    this.load_detail(Detail::Child(child_id), false, cx);
                                }
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }
    }

    pub fn set_tab(&mut self, tab: ActivityTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.tab = tab;
            self.close_detail(cx);
            self.notice = None;
            self.scroll.set_offset(point(px(0.), px(0.)));
        }
        if self.visible && !self.loading {
            self.refresh(window, cx)
        }
        cx.notify();
    }

    pub fn refresh(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        self.refresh_inner(cx);
        if let Some(detail) = self.detail.clone() {
            self.load_detail(detail, false, cx);
        }
    }

    fn refresh_inner(&mut self, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        let Some(key) = self.session.clone() else {
            return;
        };
        if self
            .latest
            .get("refresh")
            .is_some_and(|id| self.pending.contains_key(id))
        {
            return;
        }
        self.loading = true;
        self.error = None;
        self.send(ActivityCommand::Refresh { key }, "refresh", None, cx);
    }

    fn send(
        &mut self,
        command: ActivityCommand,
        slot: &'static str,
        detail: Option<Detail>,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.session.clone() else {
            return;
        };
        match self.bridge.send(Command::Activity(command)) {
            Ok(id) => {
                self.latest.insert(slot, id);
                self.pending.insert(id, Pending { key, slot, detail });
            }
            Err(message) => {
                if slot == "detail" {
                    self.detail_loading = false;
                    self.detail_error = Some(message);
                } else {
                    self.loading = false;
                    self.error = Some(message);
                }
            }
        }
        cx.notify();
    }

    pub fn receive(
        &mut self,
        request_id: u64,
        key: SessionKey,
        data: Value,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.pending.remove(&request_id) else {
            return;
        };
        if !accept_reply(&pending, &self.session, &key, request_id, &self.latest) {
            return;
        }
        match field(&data, "kind") {
            "refresh" => {
                self.loading = false;
                self.loaded = true;
                self.artifact_error = data["artifact_error"].as_str().map(str::to_owned);
                self.child_error = data["child_error"].as_str().map(str::to_owned);
                // Retain a last-good list on a source failure, alongside its
                // explicit error; unreadability is not an empty session.
                if self.artifact_error.is_none() {
                    self.artifacts = rows(&data, "artifacts")
                }
                if self.child_error.is_none() {
                    self.children = rows(&data, "children")
                }
                self.jobs = rows(&data, "jobs");
            }
            "jobs" => {
                self.jobs = rows(&data, "jobs");
                self.notice = Some("后台任务已更新".into());
            }
            "logs" | "artifact_preview" | "child_history" => {
                if pending.detail != self.detail {
                    return;
                }
                self.detail_loading = false;
                self.detail_error = None;
                self.detail_text = if field(&data, "kind") == "child_history" {
                    transcript_text(&rows(&data, "messages"))
                } else {
                    field(&data, "text").to_owned()
                };
                let html = format!(
                    "<pre>{}</pre>",
                    escape(&bounded(&self.detail_text, 200_000))
                );
                self.detail_view = Some(cx.new(|cx| TextViewState::html(&html, cx)));
            }
            "steered" => {
                self.notice = Some("指导已发送；子代理将在可接收时读取".into());
            }
            _ => self.error = Some("收到无法识别的活动响应".into()),
        }
        cx.notify();
    }

    /// Returns true only for an activity request, so the shell can avoid also
    /// presenting the same failure as an unrelated global command error.
    pub fn request_failed(&mut self, id: u64, message: &str, cx: &mut Context<Self>) -> bool {
        let Some(pending) = self.pending.remove(&id) else {
            return false;
        };
        if self.session.as_ref() != Some(&pending.key) || self.latest.get(pending.slot) != Some(&id)
        {
            return true;
        }
        if pending.slot == "detail" {
            if pending.detail == self.detail {
                self.detail_loading = false;
                self.detail_error = Some(message.to_owned());
            }
        } else {
            if pending.slot == "refresh" {
                self.loading = false
            }
            self.error = Some(message.to_owned());
        }
        cx.notify();
        true
    }

    fn load_detail(&mut self, detail: Detail, clear: bool, cx: &mut Context<Self>) {
        let Some(key) = self.session.clone() else {
            return;
        };
        if clear {
            self.detail_text.clear();
            self.detail_view = None;
        }
        self.detail = Some(detail.clone());
        self.detail_loading = true;
        self.detail_error = None;
        let command = match &detail {
            Detail::Artifact(path) => ActivityCommand::ArtifactPreview {
                key,
                path: path.clone(),
            },
            Detail::Logs(job) => ActivityCommand::Logs {
                key,
                job: job.clone(),
            },
            Detail::Child(child_id) => ActivityCommand::ChildHistory {
                key,
                child_id: *child_id,
            },
        };
        self.send(command, "detail", Some(detail), cx);
    }

    fn close_detail(&mut self, cx: &mut Context<Self>) {
        self.detail = None;
        self.detail_view = None;
        self.detail_text.clear();
        self.detail_loading = false;
        self.detail_error = None;
        self.latest.remove("detail");
        cx.notify();
    }

    fn job_control(&mut self, job: String, action: &str, cx: &mut Context<Self>) {
        let Some(key) = self.session.clone() else {
            return;
        };
        self.error = None;
        self.notice = None;
        self.send(
            ActivityCommand::JobControl {
                key,
                job,
                action: action.into(),
            },
            "control",
            None,
            cx,
        );
    }

    fn steer_child(&mut self, child_id: u64, cx: &mut Context<Self>) {
        let Some(key) = self.session.clone() else {
            return;
        };
        let text = self.steer.read(cx).value().to_string();
        if text.trim().is_empty() {
            self.error = Some("请输入指导内容".into());
            cx.notify();
            return;
        }
        self.error = None;
        self.notice = None;
        self.send(
            ActivityCommand::SteerChild {
                key,
                child_id,
                text,
            },
            "steer",
            None,
            cx,
        );
    }
}

fn accept_reply(
    pending: &Pending,
    session: &Option<SessionKey>,
    key: &SessionKey,
    id: u64,
    latest: &HashMap<&'static str, u64>,
) -> bool {
    session.as_ref() == Some(key) && &pending.key == key && latest.get(pending.slot) == Some(&id)
}
fn rows(value: &Value, key: &str) -> Vec<Value> {
    value[key].as_array().cloned().unwrap_or_default()
}
fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn bounded(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.into();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1
    }
    format!(
        "{}\n\n[仅显示前 {limit} 字节，复制可取得完整已加载内容]",
        &text[..end]
    )
}
fn transcript_text(messages: &[Value]) -> String {
    messages
        .iter()
        .map(|m| {
            let role = field(m, "role");
            let mut body = field(m, "content").to_owned();
            if let Some(calls) = m["tool_calls"].as_array() {
                for call in calls {
                    body.push_str(&format!(
                        "\n工具 {} {}\n",
                        field(&call["function"], "name"),
                        field(&call["function"], "arguments")
                    ));
                }
            }
            format!("[{role}]\n{body}")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
