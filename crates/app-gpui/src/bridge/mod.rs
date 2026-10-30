//! Typed boundary between the native view and the host's background owner.

mod activity;
mod owner;
mod settings;
pub use activity::ActivityCommand;
pub use settings::SettingsCommand;
#[cfg(test)]
mod tests;

use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionKey {
    pub root: String,
    pub id: i64,
}

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub key: SessionKey,
    pub title: String,
    pub preview: String,
    pub running: bool,
    pub pinned: bool,
    pub pinned_global: bool,
    pub updated_at: u64,
}

#[derive(Debug, Clone)]
pub struct WorkspaceRow {
    pub path: String,
    pub title: String,
    pub active: bool,
}

#[derive(Debug, Clone)]
pub struct ConfigInfo {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub workspace_dir_name: String,
    pub permission_mode: String,
    pub agent_mode: String,
    pub thinking_level: Option<String>,
    pub theme_mode: String,
    /// None means the preference could not be read; desktop delivery stays off.
    pub desktop_notifications: Option<bool>,
    /// Unavailable preferences remain disabled in the settings view.
    pub memory_preferences: Option<husk_host::prefs::MemoryPreferences>,
    pub computer_control_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct PendingApproval {
    pub key: SessionKey,
    pub request_id: u64,
    pub tool_name: String,
    pub diff: String,
    pub fuzzy: bool,
    pub child_id: Option<u64>,
    pub actions: Vec<agent_ipc::events::ApprovalAction>,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub active: Option<SessionKey>,
    pub sessions: Vec<SessionRow>,
    pub workspaces: Vec<WorkspaceRow>,
    pub history: Vec<agent_llm::types::ChatMessage>,
    /// Current live queue for the active session, including restored prompts.
    pub queued_prompts: Vec<String>,
    pub model_info: serde_json::Value,
    pub config: ConfigInfo,
    pub pending_approvals: Vec<PendingApproval>,
    /// False means retain the session's existing history and event buffer.
    pub history_loaded: bool,
    /// Number of Ui events published before this snapshot.
    pub event_sequence: u64,
}

#[derive(Debug, Clone)]
pub enum BridgeEvent {
    Activity {
        request_id: u64,
        key: SessionKey,
        data: serde_json::Value,
    },
    Snapshot {
        request_id: u64,
        snapshot: Snapshot,
    },
    /// The answer was delivered to its waiting host request.
    AnswerDelivered {
        request_id: u64,
    },
    /// A prompt was durably queued and can no longer be restored as unsent.
    SubmissionAccepted {
        request_id: u64,
    },
    /// A queue modification was persisted; later snapshot failures do not undo it.
    QueueUpdated {
        request_id: u64,
    },
    Ui {
        key: SessionKey,
        event: agent_ipc::UiEvent,
    },
    Error {
        request_id: u64,
        message: String,
    },
}

#[derive(Debug, Clone)]
pub enum Command {
    Activity(ActivityCommand),
    Settings(SettingsCommand),
    Refresh,
    NewSession,
    OpenSession(SessionKey),
    DeleteSession(SessionKey),
    SetSessionPinned {
        key: SessionKey,
        pinned: bool,
        global: bool,
    },
    RenameSession {
        key: SessionKey,
        title: String,
    },
    SwitchWorkspace(PathBuf),
    Send {
        key: SessionKey,
        text: String,
    },
    RemoveQueued {
        key: SessionKey,
        index: usize,
        expected: Vec<String>,
    },
    EditQueued {
        key: SessionKey,
        index: usize,
        expected: Vec<String>,
        text: String,
    },
    MoveQueued {
        key: SessionKey,
        from: usize,
        to: usize,
        expected: Vec<String>,
    },
    ResumeQueued {
        key: SessionKey,
    },
    Cancel {
        key: SessionKey,
    },
    Decide {
        key: SessionKey,
        request_id: u64,
        approved: bool,
        action: Option<String>,
        child_id: Option<u64>,
    },
    RevokeSessionTool {
        key: SessionKey,
        tool_name: String,
    },
    AnswerQuestion {
        key: SessionKey,
        request_id: u64,
        answer: String,
    },
    SetModel {
        key: SessionKey,
        provider: String,
        model: String,
    },
    SetFastMode {
        key: SessionKey,
        enabled: bool,
    },
    SetThinkingLevel {
        key: SessionKey,
        level: String,
    },
    SetPermissionMode {
        key: SessionKey,
        mode: String,
    },
    SetAgentMode {
        key: SessionKey,
        mode: String,
    },
    SetThemeMode(String),
    SetMemoryEnabled(bool),
    SetMemoryDistill(bool),
    SetDesktopNotifications(bool),
    ReloadConfig,
}

pub struct HostBridge {
    inner: owner::Client,
}

impl HostBridge {
    /// Starts only the owner thread; filesystem and host boot happen there.
    pub fn spawn(paths: agent_paths::AppPaths) -> io::Result<Self> {
        owner::Client::spawn(paths).map(|inner| Self { inner })
    }

    /// Receiver clones share one queue; use one consumer to preserve ordering.
    pub fn events(&self) -> async_channel::Receiver<BridgeEvent> {
        self.inner.events()
    }

    /// Success means accepted for delivery; errors after acceptance use the id.
    pub fn send(&self, command: Command) -> Result<u64, String> {
        self.inner.send(command)
    }

    /// Idempotent cancellation and detached teardown, never a thread join.
    pub fn shutdown(&self) {
        self.inner.shutdown();
    }
}

impl Drop for HostBridge {
    fn drop(&mut self) {
        self.shutdown();
    }
}
