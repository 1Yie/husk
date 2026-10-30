use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, TitleBar, WindowExt,
    button::{Button, ButtonVariant, ButtonVariants},
    dialog::DialogEntrance,
    h_flex,
    input::{Input, InputState},
    menu::{ContextMenuExt, PopupMenu, PopupMenuItem},
    resizable::{ResizableState, h_resizable, resizable_panel, v_resizable},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use husk_terminal::TerminalState;

use crate::bridge::{
    BridgeEvent, Command, HostBridge, SessionKey, SessionRow, SettingsCommand, Snapshot,
};
use crate::features::{
    activity::{ActivityPanel, ActivityTab},
    chat::ChatPanel,
    notifications::{Notice, NotificationPolicy},
    settings::{SettingsEvent, SettingsPanel},
    terminal::{TerminalDeck, TerminalDeckEvent},
    workspace::{WorkspacePanel, WorkspaceTab},
};
use crate::platform::notifications::{
    DesktopNotifications, EnqueueError, NativeNotificationOptions, Notification, NotificationError,
};
use crate::ui::icons::Keyline;
use crate::ui::{
    layout,
    motion::{self, SmoothScroll},
    popover::{Menu, Popover},
    theme,
};
mod office;
mod welcome;
use office::OfficeFiles;

actions!(
    husk,
    [
        Quit,
        NewSession,
        ToggleTerminal,
        ToggleSettings,
        ToggleSidebar,
        ToggleDock,
        OpenWorkspace,
        BackToWorkspace
    ]
);

#[derive(Clone)]
struct AppNotice {
    id: u64,
    key: SessionKey,
    notice: Notice,
    read: bool,
    created: std::time::Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DockTab {
    Tools,
    Changes,
    Artifacts,
    Jobs,
    Subagents,
}

impl DockTab {
    const ALL: [Self; 5] = [
        Self::Tools,
        Self::Changes,
        Self::Artifacts,
        Self::Jobs,
        Self::Subagents,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Tools => "工具",
            Self::Changes => "改动",
            Self::Artifacts => "产物",
            Self::Jobs => "后台",
            Self::Subagents => "子代理",
        }
    }

    fn activity(self) -> Option<ActivityTab> {
        match self {
            Self::Artifacts => Some(ActivityTab::Artifacts),
            Self::Jobs => Some(ActivityTab::Jobs),
            Self::Subagents => Some(ActivityTab::Subagents),
            _ => None,
        }
    }
}

pub struct Shell {
    bridge: Arc<HostBridge>,
    terminals: TerminalState,
    snapshot: Option<Snapshot>,
    chats: HashMap<SessionKey, Entity<ChatPanel>>,
    pending: HashMap<SessionKey, Vec<agent_ipc::UiEvent>>,
    pending_deletions: HashMap<u64, SessionKey>,
    deleted_sessions: HashSet<SessionKey>,
    notifications: Option<DesktopNotifications>,
    notification_policy: NotificationPolicy,
    app_notices: Vec<AppNotice>,
    next_notice: u64,
    terminal_panels: HashMap<String, Entity<TerminalDeck>>,
    settings: Entity<SettingsPanel>,
    workspace: Entity<WorkspacePanel>,
    activity: Entity<ActivityPanel>,
    office: OfficeFiles,
    dock_tab: DockTab,
    show_dock: bool,
    show_sidebar: bool,
    show_settings: bool,
    settings_was_open: bool,
    show_terminal: bool,
    show_all_sessions: bool,
    show_archived: bool,
    collapsed_projects: HashSet<PathBuf>,
    known_projects: HashSet<PathBuf>,
    project_scroll: ScrollHandle,
    project_scroll_motion: SmoothScroll,
    workspace_split: Entity<ResizableState>,
    terminal_split: Entity<ResizableState>,
    navigation_request: u64,
    error: Option<String>,
    quitting: bool,
    _events: Task<()>,
    quit_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub(crate) fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.show_settings = true;
        cx.notify();
    }

    pub fn new(
        bridge: Arc<HostBridge>,
        terminals: TerminalState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let events = bridge.events();
        let task = cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this
                    .update_in(cx, |this, window, cx| this.receive(event, window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let settings = cx.new(|_| SettingsPanel::new(bridge.clone()));
        let workspace = cx.new(|cx| WorkspacePanel::new(window, cx));
        workspace.update(cx, |panel, cx| panel.set_header_visible(false, cx));
        let activity = cx.new(|cx| ActivityPanel::new(bridge.clone(), window, cx));
        activity.update(cx, |panel, cx| {
            panel.set_header_visible(false, cx);
            panel.set_visible(false, window, cx);
        });
        let appearance_observer = {
            let weak = cx.weak_entity();
            window.observe_window_appearance(move |window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    if this
                        .snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.config.theme_mode == "system")
                        && let Some(snapshot) = &this.snapshot
                    {
                        apply_snapshot_appearance(snapshot, window, cx);
                    }
                });
            })
        };
        let subscriptions = vec![
            cx.observe(&workspace, |_, _, cx| cx.notify()),
            cx.observe(&activity, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &settings,
                window,
                |this, _, event: &SettingsEvent, window, cx| match event {
                    SettingsEvent::Back => {
                        this.back_to_workspace(&BackToWorkspace, window, cx);
                    }
                },
            ),
            appearance_observer,
        ];
        let notifications = match DesktopNotifications::native(NativeNotificationOptions::default())
        {
            Ok(notifications) => Some(notifications),
            Err(_) => {
                tracing::warn!("Could not start desktop notification worker");
                None
            }
        };
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let _ = weak.update(cx, |this, cx| this.request_quit(window, cx));
            false
        });
        Self {
            bridge,
            terminals,
            snapshot: None,
            chats: HashMap::new(),
            pending: HashMap::new(),
            pending_deletions: HashMap::new(),
            deleted_sessions: HashSet::new(),
            notifications,
            notification_policy: NotificationPolicy::default(),
            app_notices: Vec::new(),
            next_notice: 0,
            terminal_panels: HashMap::new(),
            settings,
            workspace,
            activity,
            office: OfficeFiles::default(),
            dock_tab: DockTab::Changes,
            show_dock: true,
            show_sidebar: true,
            show_settings: false,
            settings_was_open: false,
            show_terminal: false,
            show_all_sessions: false,
            show_archived: false,
            collapsed_projects: HashSet::new(),
            known_projects: HashSet::new(),
            project_scroll: ScrollHandle::new(),
            project_scroll_motion: SmoothScroll::default(),
            workspace_split: cx.new(|_| ResizableState::default()),
            terminal_split: cx.new(|_| ResizableState::default()),
            navigation_request: 0,
            error: None,
            quitting: false,
            _events: task,
            quit_task: None,
            _subscriptions: subscriptions,
        }
    }

    fn receive(&mut self, event: BridgeEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            BridgeEvent::Activity {
                request_id,
                key,
                data,
            } => {
                if data["kind"].as_str() == Some("workspace_links") {
                    if let Some(panel) = self.chats.get(&key) {
                        panel.update(cx, |panel, cx| {
                            panel.receive_workspace_links(request_id, &key, &data, cx)
                        });
                    }
                } else if !self.receive_office_files(request_id, &key, &data, cx) {
                    self.activity
                        .update(cx, |panel, cx| panel.receive(request_id, key, data, cx));
                }
            }
            BridgeEvent::Snapshot {
                request_id,
                snapshot,
            } => {
                // A completed settings command must not stay pending merely
                // because a newer navigation request supersedes its snapshot.
                self.settings
                    .update(cx, |settings, cx| settings.acknowledge(request_id, cx));
                if let Some(key) = self.pending_deletions.remove(&request_id) {
                    self.chats.remove(&key);
                    self.pending.remove(&key);
                    self.notification_policy.forget(&key);
                    self.app_notices.retain(|notice| notice.key != key);
                    self.deleted_sessions.insert(key);
                }
                for panel in self.chats.values() {
                    panel.update(cx, |panel, cx| {
                        panel.acknowledge_submission(request_id, window, cx)
                    });
                }
                if request_id < self.navigation_request {
                    return;
                }
                for row in &snapshot.sessions {
                    self.deleted_sessions.remove(&row.key);
                }
                let theme_changed = self.snapshot.as_ref().is_none_or(|previous| {
                    previous.config.theme_mode != snapshot.config.theme_mode
                        || previous.model_info.pointer("/settings/appearance")
                            != snapshot.model_info.pointer("/settings/appearance")
                });
                let terminal_changed = self.snapshot.as_ref().is_none_or(|previous| {
                    previous.model_info.pointer("/settings/terminal")
                        != snapshot.model_info.pointer("/settings/terminal")
                });
                let root_changed = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.active.as_ref())
                    .map(|key| &key.root)
                    != snapshot.active.as_ref().map(|key| &key.root);
                let previous_session = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.active.clone());
                for workspace in &snapshot.workspaces {
                    let path = PathBuf::from(&workspace.path);
                    if self.known_projects.insert(path.clone()) && !workspace.active {
                        self.collapsed_projects.insert(path);
                    }
                }
                if root_changed && let Some(key) = &snapshot.active {
                    self.collapsed_projects.remove(&PathBuf::from(&key.root));
                }
                if let Some(key) = &snapshot.active {
                    let key = key.clone();
                    if !self.chats.contains_key(&key) {
                        let history = &snapshot.history;
                        let events = self.pending.remove(&key).unwrap_or_default();
                        let panel = cx.new(|cx| {
                            let mut panel = ChatPanel::new(
                                key.clone(),
                                self.bridge.clone(),
                                history,
                                window,
                                cx,
                            );
                            panel.model.queued_prompts = snapshot.queued_prompts.clone();
                            for event in events {
                                panel.apply_event(&event, cx);
                            }
                            panel
                        });
                        self.chats.insert(key.clone(), panel);
                    } else if snapshot.history_loaded
                        && let Some(panel) = self.chats.get(&key)
                    {
                        panel.update(cx, |panel, cx| {
                            panel.replace_history(&snapshot.history, cx);
                            panel.model.queued_prompts = snapshot.queued_prompts.clone();
                        });
                    }
                    if let Some(panel) = self.chats.get(&key) {
                        panel.update(cx, |panel, cx| {
                            panel.set_context(&snapshot.model_info, &snapshot.config, cx);
                            if let Some(row) = snapshot.sessions.iter().find(|row| row.key == key) {
                                panel.model.running = row.running;
                            }
                            for approval in &snapshot.pending_approvals {
                                if approval.key == key {
                                    panel.apply_event(
                                        &agent_ipc::UiEvent::ApprovalRequested {
                                            request_id: approval.request_id,
                                            tool_name: approval.tool_name.clone(),
                                            diff: approval.diff.clone(),
                                            fuzzy: approval.fuzzy,
                                            actions: approval.actions.clone(),
                                            child_id: approval.child_id,
                                        },
                                        cx,
                                    );
                                }
                            }
                            cx.notify();
                        });
                        if previous_session.as_ref() != Some(&key) && !self.show_settings {
                            panel.update(cx, |panel, cx| panel.focus(window, cx));
                        }
                    }
                }
                if root_changed && let Some(key) = &snapshot.active {
                    self.workspace.update(cx, |panel, cx| {
                        panel.set_workspace(PathBuf::from(&key.root), window, cx)
                    });
                }
                self.activity.update(cx, |panel, cx| {
                    panel.set_session(snapshot.active.clone(), window, cx);
                });
                self.settings.update(cx, |settings, cx| {
                    settings.set_snapshot(snapshot.clone(), cx)
                });
                if let Some(notifications) = &self.notifications {
                    notifications
                        .set_enabled(snapshot.config.desktop_notifications.unwrap_or(false));
                }
                self.snapshot = Some(snapshot);
                self.error = None;
                if root_changed
                    || previous_session.as_ref()
                        != self
                            .snapshot
                            .as_ref()
                            .and_then(|snapshot| snapshot.active.as_ref())
                {
                    self.refresh_office_files(cx);
                }
                if theme_changed && let Some(snapshot) = &self.snapshot {
                    apply_snapshot_appearance(snapshot, window, cx);
                }
                if terminal_changed
                    && let Some(value) = self
                        .snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.model_info.pointer("/settings/terminal"))
                        .filter(|value| value.is_object())
                    && let Err(message) = crate::features::terminal::apply_preferences(value, cx)
                {
                    self.error = Some(message);
                }
                if self.show_terminal {
                    self.ensure_terminal(window, cx);
                }
            }
            BridgeEvent::AnswerDelivered { request_id }
            | BridgeEvent::SubmissionAccepted { request_id }
            | BridgeEvent::QueueUpdated { request_id } => {
                for panel in self.chats.values() {
                    panel.update(cx, |panel, cx| {
                        panel.acknowledge_submission(request_id, window, cx)
                    });
                }
            }
            BridgeEvent::Ui { key, event } => {
                if self.deleted_sessions.contains(&key) {
                    return;
                }
                self.notify_for_event(&key, &event, window, cx);
                if let Some(panel) = self.chats.get(&key) {
                    panel.update(cx, |panel, cx| panel.apply_event(&event, cx));
                } else {
                    self.pending
                        .entry(key.clone())
                        .or_default()
                        .push(event.clone());
                }
                if let Some(snapshot) = self.snapshot.as_mut()
                    && let Some(row) = snapshot.sessions.iter_mut().find(|row| row.key == key)
                {
                    match &event {
                        agent_ipc::UiEvent::StateChanged(state) => row.running = state.is_active(),
                        agent_ipc::UiEvent::AssistantMessage(text) => {
                            row.preview = text.chars().take(80).collect();
                        }
                        _ => {}
                    }
                }
                if matches!(event, agent_ipc::UiEvent::ToolCallFinished { .. }) {
                    self.workspace
                        .update(cx, |panel, cx| panel.refresh(window, cx));
                    self.refresh_office_files(cx);
                }
                if matches!(
                    event,
                    agent_ipc::UiEvent::TextDelta { .. }
                        | agent_ipc::UiEvent::ReasoningDelta { .. }
                ) {
                    return;
                }
            }
            BridgeEvent::Error {
                request_id,
                message,
            } => {
                if self.fail_office_files(request_id, &message, cx) {
                    return;
                }
                if self.activity.update(cx, |panel, cx| {
                    panel.request_failed(request_id, &message, cx)
                }) {
                    return;
                }
                self.pending_deletions.remove(&request_id);
                if self.settings.update(cx, |settings, cx| {
                    settings.request_failed(request_id, &message, cx)
                }) {
                    cx.notify();
                    return;
                }
                let active = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.active.as_ref());
                let mut failed_session = None;
                for (key, panel) in &self.chats {
                    if panel.update(cx, |panel, cx| {
                        panel.submission_failed(request_id, &message, window, cx)
                    }) {
                        failed_session = Some(key);
                        break;
                    }
                }
                if failed_session.is_none() || failed_session == active {
                    self.error = Some(message.clone());
                    if failed_session.is_none()
                        && let Some(panel) = self.active_chat()
                    {
                        panel.update(cx, |panel, cx| panel.set_error(message, cx));
                    }
                }
            }
        }
        cx.notify();
    }

    fn notify_for_event(
        &mut self,
        key: &SessionKey,
        event: &agent_ipc::UiEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let visible = window.is_window_active()
            && !self.show_settings
            && self
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.active.as_ref())
                == Some(key);
        let Some(notice) = self.notification_policy.observe(key, event, visible) else {
            return;
        };
        self.next_notice = self.next_notice.wrapping_add(1);
        self.app_notices.insert(
            0,
            AppNotice {
                id: self.next_notice,
                key: key.clone(),
                notice,
                read: false,
                created: std::time::Instant::now(),
            },
        );
        self.app_notices.truncate(100);
        let Some(notifications) = &self.notifications else {
            return;
        };
        match notifications.enqueue(Notification::new("Husk").with_safe_body(notice.body())) {
            Ok(receipt) => {
                cx.spawn(async move |_, _| {
                    if let Ok(Err(error)) = receipt.recv().await {
                        let category = match error {
                            NotificationError::Unavailable(_) => "unavailable",
                            NotificationError::PermissionDenied => "permission denied",
                            NotificationError::TimedOut => "timeout",
                            NotificationError::Backend(_) => "backend error",
                            NotificationError::BackendPanicked => "backend panic",
                        };
                        tracing::warn!(category, "Desktop notification delivery failed");
                    }
                })
                .detach();
            }
            Err(EnqueueError::Disabled) => {}
            Err(EnqueueError::QueueFull) => tracing::warn!("Desktop notification queue is full"),
            Err(EnqueueError::WorkerStopped) => {
                tracing::warn!("Desktop notification worker stopped")
            }
        }
    }

    fn active_chat(&self) -> Option<Entity<ChatPanel>> {
        self.snapshot
            .as_ref()?
            .active
            .as_ref()
            .and_then(|key| self.chats.get(key))
            .cloned()
    }

    fn back_to_workspace(
        &mut self,
        _: &BackToWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_settings = false;
        if let Some(panel) = self.active_chat() {
            panel.update(cx, |panel, cx| panel.focus(window, cx));
        }
        cx.notify();
    }

    fn command(&mut self, command: Command, navigation: bool, cx: &mut Context<Self>) {
        let deletion = match &command {
            Command::DeleteSession(key) => Some(key.clone()),
            _ => None,
        };
        match self.bridge.send(command) {
            Ok(id) => {
                if let Some(key) = deletion {
                    self.pending_deletions.insert(id, key);
                }
                if navigation {
                    self.navigation_request = id;
                    self.show_settings = false;
                }
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn open_workspace(&mut self, _: &OpenWorkspace, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择工作区".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, _, cx| {
                    this.command(Command::SwitchWorkspace(path), true, cx)
                });
            }
        })
        .detach();
    }

    fn toggle_terminal(&mut self, _: &ToggleTerminal, window: &mut Window, cx: &mut Context<Self>) {
        self.show_terminal = !self.show_terminal;
        if self.show_terminal {
            self.ensure_terminal(window, cx);
            if let Some(panel) = self.terminal_panels.get(&self.workspace_root()) {
                panel.update(cx, |panel, cx| panel.focus(window, cx));
            }
        }
        cx.notify();
    }

    fn ensure_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.workspace_root();
        if self.terminal_panels.contains_key(&root) {
            return;
        }
        let registry = self.terminals.clone();
        let owner = format!("gpui-{}", std::process::id());
        let panel =
            cx.new(|cx| TerminalDeck::new(registry, owner, PathBuf::from(&root), window, cx));
        self._subscriptions
            .push(cx.subscribe(&panel, |this, _, _: &TerminalDeckEvent, cx| {
                this.show_terminal = false;
                cx.notify();
            }));
        self.terminal_panels.insert(root, panel);
    }

    fn switch_mode(&mut self, office: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let office_root = snapshot.config.data_dir.join("office");
        let target = if office {
            Some(office_root)
        } else {
            snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.path != office_root)
                .map(|workspace| PathBuf::from(&workspace.path))
        };
        if let Some(path) = target {
            self.command(Command::SwitchWorkspace(path), true, cx);
        } else {
            self.open_workspace(&OpenWorkspace, window, cx);
        }
    }

    fn workspace_root(&self) -> String {
        self.snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.active.as_ref())
            .map(|key| key.root.clone())
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
    }

    pub fn request_quit(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.quitting {
            return;
        }
        self.quitting = true;
        self.bridge.shutdown();
        let terminals = self.terminals.clone();
        let cleanup = cx.background_executor().spawn(async move {
            terminals.shutdown();
        });
        self.quit_task = Some(cx.spawn(async move |this, cx| {
            cleanup.await;
            let _ = this.update(cx, |_, cx| cx.quit());
        }));
        cx.notify();
    }

    fn rename_session_dialog(
        &mut self,
        key: SessionKey,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(title));
        let initial_focus = Cell::new(true);
        let shell = cx.weak_entity();
        window.open_alert_dialog(cx, move |dialog, window, cx| {
            if initial_focus.replace(false) {
                input.focus_handle(cx).focus(window, cx);
            }
            let submit_input = input.clone();
            let submit_shell = shell.clone();
            let submit_key = key.clone();
            dialog
                .confirm()
                .entrance(DialogEntrance::FadeSlide)
                .title("重命名会话")
                .description("留空可在后续对话中恢复自动命名。")
                .ok_text("保存")
                .cancel_text("取消")
                .child(Input::new(&input))
                .on_ok(move |_, _, cx| {
                    let title = submit_input.read(cx).value().to_string();
                    submit_shell
                        .update(cx, |this, cx| {
                            this.command(
                                Command::RenameSession {
                                    key: submit_key.clone(),
                                    title,
                                },
                                false,
                                cx,
                            );
                        })
                        .is_ok()
                })
        });
    }

    fn delete_session_dialog(
        &mut self,
        key: SessionKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shell = cx.weak_entity();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let shell = shell.clone();
            let key = key.clone();
            dialog
                .confirm()
                .entrance(DialogEntrance::FadeSlide)
                .title("删除会话？")
                .description("会话将从应用中移除，正在运行的任务会取消。底层日志不会安全擦除。")
                .ok_text("删除")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("取消")
                .on_ok(move |_, _, cx| {
                    shell
                        .update(cx, |this, cx| {
                            this.command(Command::DeleteSession(key.clone()), true, cx);
                        })
                        .is_ok()
                })
        });
    }

    fn export_session(&mut self, key: SessionKey, window: &mut Window, cx: &mut Context<Self>) {
        let suggested = format!("husk-session-{}.md", key.id);
        let chosen = cx.prompt_for_new_path(std::path::Path::new(&key.root), Some(&suggested));
        cx.spawn_in(window, async move |this, cx| match chosen.await {
            Ok(Ok(Some(path))) => {
                let _ = this.update_in(cx, |this, _, cx| {
                    this.command(
                        Command::Settings(SettingsCommand::ExportSession { key, path }),
                        false,
                        cx,
                    );
                });
            }
            Ok(Err(error)) => {
                let _ = this.update_in(cx, |this, _, cx| {
                    this.error = Some(error.to_string());
                    cx.notify();
                });
            }
            _ => {}
        })
        .detach();
    }

    fn remove_workspace_dialog(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shell = cx.weak_entity();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let path = path.clone();
            let shell = shell.clone();
            dialog
                .confirm()
                .entrance(DialogEntrance::FadeSlide)
                .title("移除此项目？")
                .description("仅从项目列表移除，不会删除磁盘上的项目文件和会话记录。")
                .ok_text("移除")
                .cancel_text("取消")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, _, cx| {
                    shell
                        .update(cx, |this, cx| {
                            this.command(
                                Command::Settings(SettingsCommand::RemoveWorkspace(path.clone())),
                                true,
                                cx,
                            );
                        })
                        .is_ok()
                })
        });
    }

    fn render_session(&self, row: SessionRow, cx: &mut Context<Self>) -> AnyElement {
        let active = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.active.as_ref())
            == Some(&row.key);
        let can_delete = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.active.as_ref())
            .is_some_and(|active| active.root == row.key.root);
        let pinned = row.pinned;
        let pinned_global = row.pinned_global;
        let menu_row = row.clone();
        let context_row = row.clone();
        let archived = self.snapshot.as_ref().is_some_and(|snapshot| {
            archived_sessions(snapshot)
                .iter()
                .any(|archived| archived.key == row.key)
        });
        let key = row.key;
        let row_id = SharedString::from(format!("session-row:{}:{}", key.root, key.id));
        let shell = cx.weak_entity();
        let title = if row.title.is_empty() {
            "新会话".into()
        } else {
            row.title
        };
        let menu_shell = shell.clone();
        let label = format!("{}{}", if row.running { "· " } else { "" }, title);
        let accessibility_label = if pinned_global {
            format!("{label}，全局固定")
        } else if pinned {
            format!("{label}，项目固定")
        } else {
            label.clone()
        };
        div()
            .id(row_id)
            .w_full()
            .px_2()
            .pb(px(2.))
            .child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        Button::new(SharedString::from(format!(
                            "session:{}:{}",
                            key.root, key.id
                        )))
                        .ghost()
                        .w_full()
                        .h(px(32.))
                        .rounded(px(12.))
                        .text_size(px(13.))
                        .justify_start()
                        .flex_1()
                        .min_w_0()
                        .tooltip("打开会话")
                        .when(active, |this| this.bg(cx.theme().secondary))
                        .accessibility_label(accessibility_label)
                        .child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .child(div().flex_1().min_w_0().truncate().child(label))
                                .when(pinned || pinned_global, |this| {
                                    this.child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(if pinned_global { "全局" } else { "固定" }),
                                    )
                                }),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.command(Command::OpenSession(key.clone()), true, cx)
                        })),
                    )
                    .child(Menu::new(
                        SharedString::from(format!(
                            "session-menu:{}:{}",
                            menu_row.key.root, menu_row.key.id
                        )),
                        Button::new("more")
                            .ghost()
                            .small()
                            .h(px(28.))
                            .icon(Keyline::More.icon())
                            .tooltip("更多会话操作"),
                        move |menu, _, _| {
                            session_menu(menu, menu_shell, menu_row, can_delete, archived)
                        },
                    )),
            )
            .context_menu(move |menu, _, _| {
                session_menu(
                    menu,
                    shell.clone(),
                    context_row.clone(),
                    can_delete,
                    archived,
                )
            })
            .into_any_element()
    }

    fn render_notifications(&self, cx: &mut Context<Self>) -> AnyElement {
        let shell = cx.weak_entity();
        let notices = self.app_notices.clone();
        let titles = self
            .snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .sessions
                    .iter()
                    .map(|row| (row.key.clone(), row.title.clone()))
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        let unread = notices.iter().filter(|notice| !notice.read).count();
        Popover::new("notifications")
            .anchor(Anchor::BottomLeft)
            .offset(px(6.))
            .trigger(
                Button::new("notification-toggle")
                    .ghost()
                    .small()
                    .icon(Keyline::Bell.icon())
                    .tooltip("通知")
                    .accessibility_label(format!("通知，{unread} 条未读"))
                    .when(unread > 0, |button| {
                        button.label(unread.min(99).to_string())
                    }),
            )
            .w(px(288.))
            .p_0()
            .content(move |_, _, cx| {
                let theme = cx.theme().clone();
                let popup = cx.weak_entity();
                let all_read = shell.clone();
                let clear = shell.clone();
                let mut list = v_flex().w_full().child(
                    h_flex()
                        .px_3()
                        .py_2()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("通知"),
                        )
                        .when(unread > 0, |row| {
                            row.child(
                                Button::new("read-notifications")
                                    .ghost()
                                    .xsmall()
                                    .label("全部已读")
                                    .on_click(move |_, _, cx| {
                                        let _ = all_read.update(cx, |shell, cx| {
                                            for notice in &mut shell.app_notices {
                                                notice.read = true;
                                            }
                                            cx.notify();
                                        });
                                    }),
                            )
                        })
                        .when(!notices.is_empty(), |row| {
                            row.child(
                                Button::new("clear-notifications")
                                    .ghost()
                                    .xsmall()
                                    .label("清空")
                                    .on_click(move |_, _, cx| {
                                        let _ = clear.update(cx, |shell, cx| {
                                            shell.app_notices.clear();
                                            cx.notify();
                                        });
                                    }),
                            )
                        }),
                );
                if notices.is_empty() {
                    return list
                        .child(
                            div()
                                .px_3()
                                .pb_3()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("暂无通知。"),
                        )
                        .into_any_element();
                }
                let mut rows = v_flex()
                    .id("notification-list")
                    .max_h(px(256.))
                    .overflow_y_scroll()
                    .border_t_1()
                    .border_color(theme.border);
                for notice in notices {
                    let title = titles
                        .get(&notice.key)
                        .filter(|title| !title.is_empty())
                        .cloned()
                        .unwrap_or_else(|| "会话".into());
                    let popup = popup.clone();
                    let shell = shell.clone();
                    let age = notice.created.elapsed().as_secs();
                    let stamp = if age < 60 {
                        "刚刚".into()
                    } else if age < 3600 {
                        format!("{} 分钟前", age / 60)
                    } else {
                        format!("{} 小时前", age / 3600)
                    };
                    rows = rows.child(
                        Button::new(SharedString::from(format!("notice:{}", notice.id)))
                            .ghost()
                            .w_full()
                            .h_auto()
                            .px_3()
                            .py_2()
                            .justify_start()
                            .child(
                                h_flex()
                                    .w_full()
                                    .min_w_0()
                                    .items_start()
                                    .gap_2()
                                    .child(
                                        div()
                                            .size(px(6.))
                                            .mt_1p5()
                                            .rounded_full()
                                            .when(!notice.read, |dot| dot.bg(theme.primary)),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(div().text_xs().truncate().child(title))
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(theme.muted_foreground)
                                                    .child(notice.notice.body()),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(10.5))
                                                    .text_color(theme.muted_foreground)
                                                    .child(stamp),
                                            ),
                                    ),
                            )
                            .on_click(move |_, window, cx| {
                                let _ = shell.update(cx, |shell, cx| {
                                    if let Some(row) =
                                        shell.app_notices.iter_mut().find(|row| row.id == notice.id)
                                    {
                                        row.read = true;
                                    }
                                    shell.show_settings = false;
                                    shell.command(
                                        Command::OpenSession(notice.key.clone()),
                                        true,
                                        cx,
                                    );
                                });
                                let _ = popup.update(cx, |state, cx| state.dismiss(window, cx));
                            }),
                    );
                }
                list = list.child(rows);
                list.into_any_element()
            })
            .into_any_element()
    }

    fn select_dock(&mut self, tab: DockTab, window: &mut Window, cx: &mut Context<Self>) {
        self.dock_tab = tab;
        match tab {
            DockTab::Tools => self.workspace.update(cx, |panel, cx| {
                panel.set_tab(WorkspaceTab::Files, window, cx)
            }),
            DockTab::Changes => self.workspace.update(cx, |panel, cx| {
                panel.set_tab(WorkspaceTab::Changes, window, cx)
            }),
            _ => self.activity.update(cx, |panel, cx| {
                panel.set_tab(tab.activity().unwrap(), window, cx)
            }),
        }
        cx.notify();
    }

    fn render_dock(&self, width: Pixels, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let compact = width < px(300.);
        let visible = if compact {
            vec![
                DockTab::Tools,
                DockTab::Changes,
                if self.dock_tab.activity().is_some() {
                    self.dock_tab
                } else {
                    DockTab::Jobs
                },
            ]
        } else {
            DockTab::ALL.to_vec()
        };
        let mut header = h_flex()
            .h(px(36.))
            .px_1p5()
            .gap_0p5()
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme.border);
        for tab in &visible {
            let tab = *tab;
            let count = if tab == DockTab::Changes {
                self.workspace.read(cx).change_count()
            } else {
                tab.activity()
                    .map(|tab| self.activity.read(cx).count(tab))
                    .unwrap_or(0)
            };
            let label = if count > 0 {
                format!("{} {count}", tab.label())
            } else {
                tab.label().into()
            };
            header = header.child(
                Button::new(SharedString::from(format!("dock-tab:{tab:?}")))
                    .ghost()
                    .xsmall()
                    .h(px(27.))
                    .px_1p5()
                    .label(label)
                    .when(self.dock_tab == tab, |button| button.bg(theme.muted))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.select_dock(tab, window, cx)),
                    ),
            );
        }
        let shell = cx.weak_entity();
        let selected = self.dock_tab;
        header = header
            .child(div().flex_1())
            .child(Menu::new(
                "dock-more",
                Button::new("dock-more-trigger")
                    .ghost()
                    .xsmall()
                    .icon(Keyline::More.icon())
                    .tooltip("更多面板和操作"),
                move |mut menu, _, _| {
                    for tab in DockTab::ALL
                        .into_iter()
                        .filter(|tab| !visible.contains(tab))
                    {
                        let shell = shell.clone();
                        menu = menu.item(
                            PopupMenuItem::new(tab.label())
                                .checked(tab == selected)
                                .on_click(move |_, window, cx| {
                                    let _ = shell
                                        .update(cx, |this, cx| this.select_dock(tab, window, cx));
                                }),
                        );
                    }
                    menu.separator().item(PopupMenuItem::new("刷新").on_click(
                        move |_, window, cx| {
                            let _ = shell.update(cx, |this, cx| {
                                if this.dock_tab.activity().is_some() {
                                    this.activity
                                        .update(cx, |panel, cx| panel.refresh(window, cx));
                                } else {
                                    this.workspace
                                        .update(cx, |panel, cx| panel.refresh(window, cx));
                                }
                            });
                        },
                    ))
                },
            ))
            .child(
                Button::new("close-dock")
                    .ghost()
                    .xsmall()
                    .icon(Keyline::Close.icon())
                    .tooltip("收起侧面板")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_dock = false;
                        cx.notify();
                    })),
            );
        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(crate::ui::theme::panel_background(cx))
            .child(header)
            .child(div().flex_1().min_h_0().when_else(
                self.dock_tab.activity().is_some(),
                |body| body.child(self.activity.clone()),
                |body| body.child(self.workspace.clone()),
            ))
            .into_any_element()
    }
}

fn apply_snapshot_appearance(snapshot: &Snapshot, window: &Window, cx: &mut App) {
    let mode = theme::effective_mode(&snapshot.config.theme_mode, window.appearance());
    if let Some(value) = snapshot.model_info.pointer("/settings/appearance")
        && let Ok(appearance) = serde_json::from_value(value.clone())
    {
        theme::apply_appearance(mode, &appearance, cx);
    } else {
        theme::set_mode(mode, cx);
    }
}

fn session_menu(
    menu: PopupMenu,
    shell: WeakEntity<Shell>,
    row: SessionRow,
    can_delete: bool,
    archived: bool,
) -> PopupMenu {
    let rename_shell = shell.clone();
    let rename_key = row.key.clone();
    let local_shell = shell.clone();
    let local_key = row.key.clone();
    let global_shell = shell.clone();
    let global_key = row.key.clone();
    let delete_key = row.key;
    let fork_key = delete_key.clone();
    let fork_shell = shell.clone();
    let archive_key = delete_key.clone();
    let archive_shell = shell.clone();
    let export_key = delete_key.clone();
    let export_shell = shell.clone();
    menu.item(
        PopupMenuItem::new("重命名")
            .icon(Keyline::Edit.icon())
            .on_click(move |_, window, cx| {
                let _ = rename_shell.update(cx, |this, cx| {
                    this.rename_session_dialog(rename_key.clone(), row.title.clone(), window, cx);
                });
            }),
    )
    .item(
        PopupMenuItem::new("复制会话")
            .icon(Keyline::Copy.icon())
            .on_click(move |_, _, cx| {
                let _ = fork_shell.update(cx, |this, cx| {
                    this.command(
                        Command::Settings(SettingsCommand::ForkSession(fork_key.clone())),
                        true,
                        cx,
                    );
                });
            }),
    )
    .item(
        PopupMenuItem::new("导出 Markdown").on_click(move |_, window, cx| {
            let _ = export_shell.update(cx, |this, cx| {
                this.export_session(export_key.clone(), window, cx)
            });
        }),
    )
    .item(
        PopupMenuItem::new(if row.pinned {
            "取消项目固定"
        } else {
            "固定到项目"
        })
        .icon(Keyline::Pin.icon())
        .on_click(move |_, _, cx| {
            let _ = local_shell.update(cx, |this, cx| {
                this.command(
                    Command::SetSessionPinned {
                        key: local_key.clone(),
                        pinned: !row.pinned,
                        global: false,
                    },
                    false,
                    cx,
                );
            });
        }),
    )
    .item(
        PopupMenuItem::new(if row.pinned_global {
            "取消全局固定"
        } else {
            "全局固定"
        })
        .icon(Keyline::Globe.icon())
        .on_click(move |_, _, cx| {
            let _ = global_shell.update(cx, |this, cx| {
                this.command(
                    Command::SetSessionPinned {
                        key: global_key.clone(),
                        pinned: !row.pinned_global,
                        global: true,
                    },
                    false,
                    cx,
                );
            });
        }),
    )
    .separator()
    .item(
        PopupMenuItem::new(if archived {
            "取消归档"
        } else {
            "归档会话"
        })
        .icon(Keyline::Archive.icon())
        .on_click(move |_, _, cx| {
            let _ = archive_shell.update(cx, |this, cx| {
                this.command(
                    Command::Settings(SettingsCommand::SetSessionArchived {
                        key: archive_key.clone(),
                        archived: !archived,
                    }),
                    false,
                    cx,
                );
            });
        }),
    )
    .when(can_delete, |menu| {
        menu.separator().item(
            PopupMenuItem::new("删除会话")
                .icon(Keyline::Trash.icon())
                .on_click(move |_, window, cx| {
                    let _ = shell.update(cx, |this, cx| {
                        this.delete_session_dialog(delete_key.clone(), window, cx);
                    });
                }),
        )
    })
}

fn archived_sessions(snapshot: &Snapshot) -> Vec<SessionRow> {
    snapshot
        .model_info
        .pointer("/settings/archives")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            Some(SessionRow {
                key: SessionKey {
                    root: row["root"].as_str()?.to_owned(),
                    id: row["id"].as_i64()?,
                },
                title: row["title"].as_str().unwrap_or("").to_owned(),
                preview: row["preview"].as_str().unwrap_or("").to_owned(),
                running: false,
                pinned: row["pinned"].as_bool().unwrap_or(false),
                pinned_global: row["pinned_global"].as_bool().unwrap_or(false),
                updated_at: row["updated_at"].as_u64().unwrap_or(0),
            })
        })
        .collect()
}

fn recent_sessions(rows: &[SessionRow], expanded: bool) -> (Vec<SessionRow>, bool) {
    let mut rows = rows.to_vec();
    rows.sort_by(|a, b| {
        b.pinned_global
            .cmp(&a.pinned_global)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| a.key.root.cmp(&b.key.root))
            .then_with(|| b.key.id.cmp(&a.key.id))
    });
    let pinned = rows.iter().take_while(|row| row.pinned_global).count();
    let has_more = rows.len().saturating_sub(pinned) > 6;
    if !expanded {
        rows.truncate(pinned + 6);
    }
    (rows, has_more)
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        if self.show_settings {
            if !self.settings_was_open {
                self.settings_was_open = true;
                self.settings.update(cx, |panel, cx| panel.refresh(cx));
            }
            self.activity
                .update(cx, |panel, cx| panel.set_visible(false, window, cx));
            return div()
                .id("settings-route")
                .key_context("Husk HuskSettings")
                .size_full()
                .relative()
                .bg(theme.background)
                .text_color(theme.foreground)
                .on_action(cx.listener(|this, _: &Quit, window, cx| this.request_quit(window, cx)))
                .on_action(cx.listener(Self::back_to_workspace))
                .on_action(cx.listener(|this, _: &ToggleSettings, window, cx| {
                    this.back_to_workspace(&BackToWorkspace, window, cx)
                }))
                .child(self.settings.clone())
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left(px(layout::SIDEBAR_WIDTH))
                        .right_0()
                        .h(px(layout::HEADER_HEIGHT))
                        .child(
                            TitleBar::new()
                                .h(px(layout::HEADER_HEIGHT))
                                .bg(rgba(0x00000000))
                                .border_0()
                                .on_close_window(
                                    cx.listener(|this, _, window, cx| {
                                        this.request_quit(window, cx)
                                    }),
                                ),
                        ),
                )
                .with_animation(
                    "settings-route-enter",
                    Animation::new(motion::DIALOG_DURATION)
                        .with_easing(gpui_kit::base::animation::ease_out_cubic),
                    |surface, progress| surface.opacity(progress).top(px(4. * (1. - progress))),
                )
                .into_any_element();
        }
        self.settings_was_open = false;
        let sidebar_progress = motion::panel_progress(
            "sidebar-reveal",
            self.show_sidebar && !self.show_settings,
            window,
            cx,
        );
        let dock_target =
            self.show_dock && !self.show_settings && window.viewport_size().width >= px(1000.);
        let dock_progress = motion::panel_progress("dock-reveal", dock_target, window, cx);
        self.activity.update(cx, |panel, cx| {
            panel.set_visible(
                dock_target && self.dock_tab.activity().is_some(),
                window,
                cx,
            );
        });
        let terminal_progress = motion::panel_progress(
            "terminal-reveal",
            self.show_terminal && !self.show_settings,
            window,
            cx,
        );
        self.project_scroll_motion.sample(
            "sidebar-project-scroll",
            &self.project_scroll,
            window,
            cx,
        );
        let ready = self.snapshot.is_some() && !self.quitting;
        let title = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| {
                snapshot.active.as_ref().and_then(|key| {
                    snapshot
                        .sessions
                        .iter()
                        .find(|row| row.key == *key)
                        .map(|row| row.title.clone())
                })
            })
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "Husk".into());
        let root = self.workspace_root();
        let active = if self.show_settings {
            None
        } else {
            self.active_chat()
        };
        let office = self
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| root == snapshot.config.data_dir.join("office"));
        let (recent, has_more_sessions) = self
            .snapshot
            .as_ref()
            .map(|snapshot| {
                let office_root = snapshot.config.data_dir.join("office");
                let mut rows = snapshot
                    .sessions
                    .iter()
                    .filter(|row| {
                        if office {
                            std::path::Path::new(&row.key.root) == office_root
                        } else {
                            std::path::Path::new(&row.key.root) != office_root
                        }
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if office && self.show_archived {
                    rows.extend(archived_sessions(snapshot));
                }
                recent_sessions(&rows, self.show_all_sessions)
            })
            .unwrap_or_default();
        let archived_count = self
            .snapshot
            .as_ref()
            .map(archived_sessions)
            .map_or(0, |rows| rows.len());
        let recent_count = recent.len();
        let recent_target_height =
            recent_count.min(if self.show_all_sessions { 10 } else { 6 }) as f32 * 34.;
        let recent_height = px(gpui_kit::base::motion::transition(
            "recent-session-height",
            recent_target_height,
            motion::reveal_transition(self.show_all_sessions),
            window,
            cx,
        ));
        let recent_list = uniform_list(
            "recent-sessions",
            recent_count,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                recent
                    .iter()
                    .skip(range.start)
                    .take(range.len())
                    .cloned()
                    .map(|row| this.render_session(row, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .h(recent_height);
        let workspaces = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.workspaces.clone())
            .unwrap_or_default();
        let usage = active
            .as_ref()
            .map(|panel| panel.read(cx).model.usage)
            .unwrap_or_default();
        let tools = active
            .as_ref()
            .map(|panel| {
                panel
                    .read(cx)
                    .model
                    .entries
                    .iter()
                    .filter(|entry| entry.kind == crate::features::chat::model::EntryKind::Tool)
                    .count()
            })
            .unwrap_or(0);
        let branch = if self.show_settings {
            None
        } else {
            self.workspace.read(cx).branch()
        };
        let mut header = h_flex()
            .flex_1()
            .min_w_0()
            .gap_3()
            .child(
                Button::new("sidebar-toggle")
                    .ghost()
                    .small()
                    .icon(
                        if self.show_settings {
                            Keyline::Back
                        } else {
                            Keyline::PanelLeft
                        }
                        .icon(),
                    )
                    .tooltip(if self.show_settings {
                        "返回工作区"
                    } else {
                        "显示或隐藏侧栏"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.show_settings {
                            this.show_settings = false;
                        } else {
                            this.show_sidebar = !this.show_sidebar;
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(if self.show_settings {
                        "设置".into()
                    } else {
                        title
                    }),
            );
        if let Some(branch) = branch {
            header = header.child(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(Keyline::Branch.icon().small())
                    .child(branch),
            );
        }
        if tools != 0 {
            header = header.child(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(Keyline::Zap.icon().small())
                    .child(format!("{tools} 调用")),
            );
        }
        if usage.prompt_tokens != 0 || usage.completion_tokens != 0 {
            let total = u64::from(usage.prompt_tokens) + u64::from(usage.completion_tokens);
            header = header.child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{:.1}K tok", total as f64 / 1000.)),
            );
            if usage.cached_tokens != 0 && usage.prompt_tokens != 0 {
                header = header.child(div().text_xs().text_color(theme.muted_foreground).child(
                    format!(
                        "缓存 {:.1}%",
                        f64::from(usage.cached_tokens) * 100. / f64::from(usage.prompt_tokens)
                    ),
                ));
            }
            if usage.context_window != 0 {
                header = header.child(div().text_xs().text_color(theme.muted_foreground).child(
                    format!(
                        "{:.0}%",
                        total as f64 * 100. / f64::from(usage.context_window)
                    ),
                ));
            }
        }
        header = header.when(!self.show_settings, |header| {
            header.child(
                Button::new("dock-toggle")
                    .ghost()
                    .small()
                    .icon(Keyline::PanelRight.icon())
                    .tooltip("显示或隐藏改动面板")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_dock = !this.show_dock;
                        cx.notify();
                    })),
            )
        });
        let chat = div().size_full().min_w_0().min_h_0().when_else(
            self.show_settings,
            |this| this.child(self.settings.clone()),
            |this| {
                this.when_else(
                    active.is_some(),
                    |this| this.child(active.unwrap()),
                    |this| {
                        if ready && !self.quitting {
                            this.child(self.workspace_welcome(cx))
                        } else {
                            this.flex()
                                .items_center()
                                .justify_center()
                                .child(if self.quitting {
                                    "正在关闭会话与终端…"
                                } else if ready {
                                    "打开工作区或新建会话"
                                } else {
                                    "正在启动…"
                                })
                        }
                    },
                )
            },
        );
        let dock_width = self
            .workspace_split
            .read(cx)
            .sizes()
            .get(1)
            .copied()
            .filter(|size| *size >= px(240.))
            .unwrap_or(px(layout::DOCK_WIDTH));
        let terminal_height = self
            .terminal_split
            .read(cx)
            .sizes()
            .get(1)
            .copied()
            .filter(|size| *size >= px(100.))
            .unwrap_or(px(layout::TERMINAL_HEIGHT));
        let available_width = window.viewport_size().width
            - window.client_inset().unwrap_or(px(0.)) * 2.
            - px(layout::SIDEBAR_WIDTH * sidebar_progress);
        let chat_width = (available_width - dock_width * dock_progress).max(px(300.));
        let dock = self.render_dock(dock_width, cx);
        let upper = if dock_progress == 1. {
            h_resizable("workspace-split")
                .with_state(&self.workspace_split)
                .child(
                    resizable_panel()
                        .size(chat_width)
                        .size_range(px(300.)..px(1800.))
                        .child(chat),
                )
                .child(
                    resizable_panel()
                        .size(dock_width)
                        .size_range(px(240.)..px(600.))
                        .fixed()
                        .child(
                            div()
                                .size_full()
                                .border_l_1()
                                .border_color(theme.border)
                                .child(dock),
                        ),
                )
                .into_any_element()
        } else {
            h_flex()
                .size_full()
                .min_w_0()
                .child(div().flex_1().min_w_0().h_full().child(chat))
                .when(dock_progress > 0., |this| {
                    this.child(
                        div()
                            .w(dock_width * dock_progress)
                            .h_full()
                            .flex_shrink_0()
                            .overflow_hidden()
                            .child(
                                div()
                                    .w(dock_width)
                                    .h_full()
                                    .border_l_1()
                                    .border_color(theme.border)
                                    .child(dock),
                            ),
                    )
                })
                .into_any_element()
        };
        let contents = if terminal_progress > 0. {
            if let Some(panel) = self.terminal_panels.get(&root) {
                if terminal_progress == 1. {
                    v_resizable("terminal-split")
                        .with_state(&self.terminal_split)
                        .child(
                            resizable_panel()
                                .size_range(px(180.)..px(1600.))
                                .child(upper),
                        )
                        .child(
                            resizable_panel()
                                .size(terminal_height)
                                .size_range(px(100.)..px(700.))
                                .fixed()
                                .child(
                                    div()
                                        .size_full()
                                        .border_t_1()
                                        .border_color(theme.border)
                                        .child(panel.clone()),
                                ),
                        )
                        .into_any_element()
                } else {
                    v_flex()
                        .size_full()
                        .min_h_0()
                        .child(div().flex_1().min_h_0().child(upper))
                        .child(
                            div()
                                .h(terminal_height * terminal_progress)
                                .w_full()
                                .flex_shrink_0()
                                .overflow_hidden()
                                .child(
                                    div()
                                        .h(terminal_height)
                                        .w_full()
                                        .border_t_1()
                                        .border_color(theme.border)
                                        .child(panel.clone()),
                                ),
                        )
                        .into_any_element()
                }
            } else {
                upper.into_any_element()
            }
        } else {
            upper.into_any_element()
        };
        let mut projects = v_flex().w_full();
        for workspace in workspaces {
            if self
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| workspace.path == snapshot.config.data_dir.join("office"))
            {
                continue;
            }
            let path = PathBuf::from(&workspace.path);
            let expanded = !self.collapsed_projects.contains(&path);
            let mut rows = self
                .snapshot
                .as_ref()
                .map(|snapshot| {
                    snapshot
                        .sessions
                        .iter()
                        .filter(|row| std::path::Path::new(&row.key.root) == path)
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if self.show_archived
                && let Some(snapshot) = &self.snapshot
            {
                rows.extend(
                    archived_sessions(snapshot)
                        .into_iter()
                        .filter(|row| std::path::Path::new(&row.key.root) == path),
                );
            }
            let count = rows.len();
            let toggle_path = path.clone();
            let switch_path = path.clone();
            let menu_path = path;
            let menu_shell = cx.weak_entity();
            projects = projects.child(
                h_flex()
                    .px_2()
                    .py_1()
                    .gap_1()
                    .child(
                        Button::new(SharedString::from(format!("workspace:{}", workspace.path)))
                            .ghost()
                            .flex_1()
                            .min_w_0()
                            .h(px(32.))
                            .justify_start()
                            .accessibility_label(format!(
                                "{}，{}，{count} 个会话",
                                workspace.title,
                                if expanded { "展开" } else { "收起" }
                            ))
                            .tooltip(workspace.path.clone())
                            .child(
                                h_flex()
                                    .w_full()
                                    .min_w_0()
                                    .gap_1p5()
                                    .child(
                                        if expanded {
                                            Keyline::ChevronDown
                                        } else {
                                            Keyline::ChevronRight
                                        }
                                        .icon()
                                        .xsmall(),
                                    )
                                    .child(
                                        if expanded {
                                            Keyline::FolderOpen
                                        } else {
                                            Keyline::Folder
                                        }
                                        .icon()
                                        .small(),
                                    )
                                    .child(
                                        div().flex_1().min_w_0().truncate().child(workspace.title),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .child(count.to_string()),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.collapsed_projects.remove(&toggle_path) {
                                    this.collapsed_projects.insert(toggle_path.clone());
                                }
                                cx.notify();
                            })),
                    )
                    .child(Menu::new(
                        SharedString::from(format!("workspace-menu:{}", workspace.path)),
                        Button::new("project-more")
                            .ghost()
                            .small()
                            .icon(Keyline::More.icon())
                            .tooltip("项目操作"),
                        move |menu, _, _| {
                            let switch_shell = menu_shell.clone();
                            let remove_shell = menu_shell.clone();
                            let remove_path = menu_path.to_string_lossy().into_owned();
                            let new_shell = menu_shell;
                            let new_path = menu_path.clone();
                            let reveal_path = menu_path.clone();
                            menu.item(
                                PopupMenuItem::new("切换到此项目")
                                    .icon(Keyline::FolderOpen.icon())
                                    .on_click(move |_, _, cx| {
                                        let _ = switch_shell.update(cx, |this, cx| {
                                            this.command(
                                                Command::SwitchWorkspace(switch_path.clone()),
                                                true,
                                                cx,
                                            );
                                        });
                                    }),
                            )
                            .item(
                                PopupMenuItem::new("新建会话")
                                    .icon(Keyline::Plus.icon())
                                    .on_click(move |_, _, cx| {
                                        let _ = new_shell.update(cx, |this, cx| {
                                            this.command(
                                                Command::SwitchWorkspace(new_path.clone()),
                                                true,
                                                cx,
                                            );
                                            this.command(Command::NewSession, true, cx);
                                        });
                                    }),
                            )
                            .separator()
                            .item(
                                PopupMenuItem::new("在文件夹中显示")
                                    .icon(Keyline::Folder.icon())
                                    .on_click(move |_, _, cx| cx.reveal_path(&reveal_path)),
                            )
                            .item(
                                PopupMenuItem::new("复制项目路径")
                                    .icon(Keyline::Copy.icon())
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            menu_path.to_string_lossy().into_owned(),
                                        ));
                                    }),
                            )
                            .separator()
                            .item(
                                PopupMenuItem::new("移除项目")
                                    .icon(Keyline::Trash.icon())
                                    .on_click(move |_, window, cx| {
                                        let _ = remove_shell.update(cx, |this, cx| {
                                            this.remove_workspace_dialog(
                                                remove_path.clone(),
                                                window,
                                                cx,
                                            )
                                        });
                                    }),
                            )
                        },
                    )),
            );
            let progress = gpui_kit::base::motion::transition(
                SharedString::from(format!("project-reveal:{}", workspace.path)),
                if expanded { 1_f32 } else { 0_f32 },
                motion::reveal_transition(expanded),
                window,
                cx,
            );
            if progress > 0. && count > 0 {
                let list_height = px((count as f32 * 34.).min(340.));
                projects = projects.child(
                    div()
                        .w_full()
                        .h(list_height * progress)
                        .overflow_hidden()
                        .child(
                            uniform_list(
                                SharedString::from(format!("project-sessions:{}", workspace.path)),
                                count,
                                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                                    rows.iter()
                                        .skip(range.start)
                                        .take(range.len())
                                        .cloned()
                                        .map(|row| this.render_session(row, cx))
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .h(list_height),
                        ),
                );
            }
        }
        let projects = if office {
            self.render_office_files(window, cx)
        } else {
            projects.into_any_element()
        };
        h_flex()
            .key_context("Husk")
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .on_action(cx.listener(|this, _: &Quit, window, cx| this.request_quit(window, cx)))
            .on_action(cx.listener(|this, _: &NewSession, _, cx| {
                this.command(Command::NewSession, true, cx)
            }))
            .on_action(cx.listener(Self::toggle_terminal))
            .on_action(cx.listener(Self::open_workspace))
            .on_action(cx.listener(|this, _: &ToggleDock, _, cx| {
                this.show_dock = !this.show_dock;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.show_sidebar = !this.show_sidebar;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleSettings, _, cx| {
                this.show_settings = !this.show_settings;
                cx.notify();
            }))
            .when(sidebar_progress > 0., |this| {
                this.child(
                    div()
                        .w(px(layout::SIDEBAR_WIDTH * sidebar_progress))
                        .h_full()
                        .flex_shrink_0()
                        .overflow_hidden()
                        .child(
                            v_flex()
                                .w(px(layout::SIDEBAR_WIDTH))
                                .h_full()
                                .bg(theme.sidebar)
                                .border_r_1()
                                .border_color(theme.border)
                                .child(
                                    h_flex()
                                        .h(px(layout::HEADER_HEIGHT))
                                        .px_3()
                                        .gap_2()
                                        .border_b_1()
                                        .border_color(theme.border)
                                        .child(img("images/husk.png").size(px(13.)))
                                        .child(
                                            div().font_weight(FontWeight::SEMIBOLD).child("Husk"),
                                        )
                                        .child(div().flex_1())
                                        .child(
                                            h_flex()
                                                .gap_1()
                                                .rounded_lg()
                                                .bg(theme.secondary)
                                                .child(
                                                    Button::new("coding-mode")
                                                        .ghost()
                                                        .small()
                                                        .icon(Keyline::Code.icon())
                                                        .label("编程")
                                                        .when(!office, |this| {
                                                            this.bg(theme.background)
                                                        })
                                                        .disabled(!ready)
                                                        .on_click(cx.listener(
                                                            |this, _, window, cx| {
                                                                this.switch_mode(false, window, cx)
                                                            },
                                                        )),
                                                )
                                                .child(
                                                    Button::new("office-mode")
                                                        .ghost()
                                                        .small()
                                                        .icon(Keyline::File.icon())
                                                        .label("工作")
                                                        .when(office, |this| {
                                                            this.bg(theme.background)
                                                        })
                                                        .disabled(!ready)
                                                        .on_click(cx.listener(
                                                            |this, _, window, cx| {
                                                                this.switch_mode(true, window, cx)
                                                            },
                                                        )),
                                                ),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .h(px(36.))
                                        .px_4()
                                        .justify_between()
                                        .child(
                                            div()
                                                .text_color(theme.muted_foreground)
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child("会话"),
                                        )
                                        .when(
                                            office && (archived_count > 0 || self.show_archived),
                                            |header| {
                                                header.child(
                                                    Button::new("office-archived-sessions")
                                                        .ghost()
                                                        .xsmall()
                                                        .label(if self.show_archived {
                                                            "隐藏归档".into()
                                                        } else {
                                                            format!("归档 {archived_count}")
                                                        })
                                                        .on_click(cx.listener(|this, _, _, cx| {
                                                            this.show_archived =
                                                                !this.show_archived;
                                                            cx.notify();
                                                        })),
                                                )
                                            },
                                        )
                                        .child(div().flex_1())
                                        .when(has_more_sessions, |this| {
                                            this.child(
                                                Button::new("expand-sessions")
                                                    .ghost()
                                                    .small()
                                                    .label(if self.show_all_sessions {
                                                        "收起"
                                                    } else {
                                                        "全部"
                                                    })
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.show_all_sessions =
                                                            !this.show_all_sessions;
                                                        cx.notify();
                                                    })),
                                            )
                                        })
                                        .child(
                                            Button::new("new-session")
                                                .ghost()
                                                .small()
                                                .icon(Keyline::Plus.icon())
                                                .tooltip("新建会话")
                                                .disabled(!ready)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.command(Command::NewSession, true, cx)
                                                })),
                                        ),
                                )
                                .child(recent_list)
                                .child(
                                    h_flex()
                                        .h(px(36.))
                                        .px_4()
                                        .justify_between()
                                        .child(
                                            div()
                                                .text_color(theme.muted_foreground)
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(if office { "产物" } else { "项目" }),
                                        )
                                        .when(
                                            !office && (archived_count > 0 || self.show_archived),
                                            |header| {
                                                header.child(
                                                    Button::new("show-archived")
                                                        .ghost()
                                                        .xsmall()
                                                        .label(if self.show_archived {
                                                            "隐藏归档".into()
                                                        } else {
                                                            format!("归档 {archived_count}")
                                                        })
                                                        .on_click(cx.listener(|this, _, _, cx| {
                                                            this.show_archived =
                                                                !this.show_archived;
                                                            cx.notify();
                                                        })),
                                                )
                                            },
                                        )
                                        .when(office, |header| {
                                            header.child(
                                                Button::new("refresh-office-files")
                                                    .ghost()
                                                    .xsmall()
                                                    .label("刷新")
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.refresh_office_files(cx)
                                                    })),
                                            )
                                        })
                                        .when(!office, |header| {
                                            header.child(
                                                Button::new("workspace")
                                                    .ghost()
                                                    .small()
                                                    .icon(Keyline::Folder.icon())
                                                    .tooltip("打开工作区")
                                                    .disabled(!ready)
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            this.open_workspace(
                                                                &OpenWorkspace,
                                                                window,
                                                                cx,
                                                            )
                                                        },
                                                    )),
                                            )
                                        }),
                                )
                                .child(
                                    div()
                                        .id("sidebar-projects")
                                        .flex_1()
                                        .min_h_0()
                                        .overflow_y_scroll()
                                        .track_scroll(&self.project_scroll)
                                        .on_scroll(cx.listener(|this, event, _, cx| {
                                            if this
                                                .project_scroll_motion
                                                .on_wheel(event, &this.project_scroll)
                                            {
                                                cx.stop_propagation();
                                                cx.notify();
                                            }
                                        }))
                                        .child(projects),
                                )
                                .child(
                                    h_flex()
                                        .p_2()
                                        .gap_2()
                                        .child(
                                            Button::new("settings")
                                                .ghost()
                                                .small()
                                                .icon(Keyline::Settings.icon())
                                                .tooltip("设置")
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.show_settings = !this.show_settings;
                                                    cx.notify();
                                                })),
                                        )
                                        .child(self.render_notifications(cx))
                                        .child(div().flex_1())
                                        .child(
                                            Button::new("terminal-toggle")
                                                .ghost()
                                                .small()
                                                .icon(Keyline::Terminal.icon())
                                                .tooltip("显示或隐藏终端")
                                                .disabled(!ready)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.toggle_terminal(
                                                        &ToggleTerminal,
                                                        window,
                                                        cx,
                                                    )
                                                })),
                                        ),
                                ),
                        ),
                )
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(
                        TitleBar::new()
                            .h(px(layout::HEADER_HEIGHT))
                            .border_b_1()
                            .border_color(theme.border)
                            .on_close_window(
                                cx.listener(|this, _, window, cx| this.request_quit(window, cx)),
                            )
                            .child(header),
                    )
                    .when_some(self.error.clone(), |this, error| {
                        this.child(
                            div()
                                .px_4()
                                .py_2()
                                .text_sm()
                                .text_color(theme.danger)
                                .child(error),
                        )
                    })
                    .child(div().flex_1().min_h_0().child(contents)),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionKey, SessionRow, recent_sessions};

    fn row(id: i64, pinned_global: bool, updated_at: u64) -> SessionRow {
        SessionRow {
            key: SessionKey {
                root: format!("workspace-{id}"),
                id,
            },
            title: format!("session-{id}"),
            preview: String::new(),
            running: false,
            pinned: false,
            pinned_global,
            updated_at,
        }
    }

    #[test]
    fn expanded_recent_list_exposes_all_global_pins_before_unpinned_rows() {
        let mut rows: Vec<_> = (0..8).map(|i| row(i, true, i as u64)).collect();
        rows.push(row(8, false, 100));
        let (collapsed, has_more) = recent_sessions(&rows, false);
        assert!(!has_more);
        assert_eq!(collapsed.len(), 9);
        assert!(collapsed[..8].iter().all(|row| row.pinned_global));

        let (expanded, has_more) = recent_sessions(&rows, true);
        assert!(!has_more);
        assert_eq!(expanded.len(), 9);
        assert_eq!(expanded[0].key.id, 7);
        assert_eq!(expanded[7].key.id, 0);
        assert_eq!(expanded[8].key.id, 8);
    }

    #[test]
    fn small_recent_list_has_no_expand_control() {
        let (rows, has_more) = recent_sessions(&[row(1, false, 1)], true);
        assert_eq!(rows.len(), 1);
        assert!(!has_more);
    }
}
