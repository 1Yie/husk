//! Linked workspaces — the host's one write path for directory grants.
//!
//! Two layers hold grants (workspace preferences, and a session's own
//! `{id}.links.json`), and three callers write them: the IPC ops behind the
//! composer menu, the `link_workspace` tool, and the approval card a refusal
//! turns into. They all reach the same [`LinkHub`], so the rules cannot drift
//! apart:
//!
//! - a grant is validated before it is stored ([`crate::link_policy::validate`])
//!   and validated again when it is read back — a hand-edited prefs file, an
//!   older build, or a directory replaced by a symlink to `~/.ssh` must not
//!   become reachable just because it is on disk;
//! - every write is a locked read-modify-write, so a grant from the approval
//!   card cannot overwrite one the composer just made;
//! - a change refreshes **every live session of that workspace**, not only the
//!   one that asked. Otherwise `取消关联` would leave another window writing to
//!   a directory the user just revoked.
//!
//! The hub is per workspace and shared by `Arc`, resolved through
//! [`hub_for`] — the manager and each actor build their own view of it without
//! threading one more field through `SessionConfig`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use agent_capabilities::{LinkedRoot, LinkedRoots, RootAccess};
use agent_paths::AppPaths;
use agent_store::{LinkedRootPref, SessionStore};

use crate::link_policy;

/// Which layer a grant lands in, named by the action id the approval card
/// carries. `Session` dies with the conversation; `Workspace` is inherited by
/// every session of that workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Session,
    Workspace,
}

impl Layer {
    /// The kernel names the two scopes every host that links directories
    /// offers (`link:session` / `link:workspace`). An unknown id is not a
    /// grant at all — refusing beats guessing which layer the user meant.
    pub fn from_action(action: &str) -> Option<Self> {
        match action {
            "link:session" => Some(Self::Session),
            "link:workspace" => Some(Self::Workspace),
            _ => None,
        }
    }
}

/// One live session of the workspace, as the hub sees it.
struct Live {
    id: i64,
    roots: Weak<LinkedRoots>,
    notices: tokio::sync::mpsc::Sender<String>,
}

/// The grants of one workspace, and the sessions reaching through them.
pub struct LinkHub {
    root: PathBuf,
    paths: AppPaths,
    live: Mutex<Vec<Live>>,
}

impl LinkHub {
    /// The hub for `root`, shared with everyone else asking for the same
    /// workspace. A `Weak` registry: a hub nobody holds any more is rebuilt
    /// (with no sessions in it) rather than kept alive by the map.
    pub fn for_workspace(root: &Path, paths: &AppPaths) -> Arc<Self> {
        static HUBS: OnceLock<Mutex<HashMap<PathBuf, Weak<LinkHub>>>> = OnceLock::new();
        let canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let map = HUBS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut map = map.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(hub) = map.get(&canon).and_then(Weak::upgrade) {
            return hub;
        }
        let hub = Arc::new(Self {
            root: canon.clone(),
            paths: paths.clone(),
            live: Mutex::new(Vec::new()),
        });
        // Workspaces come and go over a long-lived process; drop the hubs
        // nobody holds any more instead of letting the map grow by one entry
        // per workspace ever opened.
        if map.len() > 32 {
            map.retain(|_, weak| weak.strong_count() > 0);
        }
        map.insert(canon, Arc::downgrade(&hub));
        hub
    }

    /// The workspace this hub governs.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Track a live session's handle. Called once per actor, at spawn: the
    /// handle is the same `Arc` its tool context holds, so [`Self::refresh`]
    /// reaches the tools of a session that is mid-turn.
    pub fn register(&self, id: i64, roots: &Arc<LinkedRoots>, notices: &tokio::sync::mpsc::Sender<String>) {
        let mut live = self.lock();
        live.retain(|l| l.id != id && l.roots.strong_count() > 0);
        live.push(Live {
            id,
            roots: Arc::downgrade(roots),
            notices: notices.clone(),
        });
    }

    /// What `id` may reach right now: both layers, validated and normalized.
    pub fn effective(&self, id: i64) -> Vec<LinkedRoot> {
        let Some(store) = self.store() else {
            return Vec::new();
        };
        let mut all = self.kept(store.load_prefs().linked_roots.iter());
        all.extend(self.kept(store.load_session_links(id).iter()));
        link_policy::normalize(all)
    }

    /// The two layers as the composer lists them, each already validated and
    /// normalized — what the pane shows is exactly what the tools reach.
    pub fn layers(&self, id: i64) -> (Vec<LinkedRoot>, Vec<LinkedRoot>) {
        let Some(store) = self.store() else {
            return (Vec::new(), Vec::new());
        };
        (
            link_policy::normalize(self.kept(store.load_prefs().linked_roots.iter())),
            link_policy::normalize(self.kept(store.load_session_links(id).iter())),
        )
    }

    /// Grant `path` to session `id`. The path is folded to the directory a
    /// grant should name (a card names a file; a grant is a directory), then
    /// validated, so no caller can store something [`Self::effective`] would
    /// drop again.
    pub fn grant(
        &self,
        id: i64,
        path: &str,
        access: &str,
        layer: Layer,
    ) -> Result<Vec<LinkedRoot>, String> {
        let target = link_policy::grant_target(Path::new(path));
        let canon = link_policy::validate(&target, &self.root, &self.paths)?;
        let access = link_policy::access_from_label(access);
        let store = self.store().ok_or("会话存储不可用")?;
        match layer {
            Layer::Workspace => {
                store
                    .update_prefs(|prefs| set_link(&mut prefs.linked_roots, &canon, access))
                    .map_err(|e| e.to_string())?;
            }
            Layer::Session => {
                store
                    .update_session_links(id, |links| set_link(links, &canon, access))
                    .map_err(|e| e.to_string())?;
            }
        }
        self.refresh();
        Ok(self.effective(id))
    }

    /// Drop `path` from **both** layers — "取消关联" means this session no
    /// longer reaches it, whichever layer granted it.
    pub fn revoke(&self, id: i64, path: &Path) -> Result<Vec<LinkedRoot>, String> {
        let store = self.store().ok_or("会话存储不可用")?;
        // Counted from the stored rows, not from the reachable list: an entry
        // that stopped validating is still a row the user is entitled to
        // remove, and removing it must not look like "nothing happened".
        let mut removed = 0usize;
        store
            .update_prefs(|prefs| {
                let before = prefs.linked_roots.len();
                prefs
                    .linked_roots
                    .retain(|l| !link_policy::same_path(&l.path, path));
                removed += before - prefs.linked_roots.len();
            })
            .map_err(|e| e.to_string())?;
        store
            .update_session_links(id, |links| {
                let before = links.len();
                links.retain(|l| !link_policy::same_path(&l.path, path));
                removed += before - links.len();
            })
            .map_err(|e| e.to_string())?;
        if removed == 0 {
            return Err(format!("{} 不是已关联的工作区", path.display()));
        }
        self.refresh();
        Ok(self.effective(id))
    }

    /// Switch one link between read and read-write, in whichever layer holds
    /// it (both, when both do — they are the same directory).
    pub fn set_access(
        &self,
        id: i64,
        path: &Path,
        access: &str,
    ) -> Result<Vec<LinkedRoot>, String> {
        let store = self.store().ok_or("会话存储不可用")?;
        let access = link_policy::access_from_label(access);
        let mut touched = 0usize;
        store
            .update_prefs(|prefs| {
                for l in prefs.linked_roots.iter_mut() {
                    if link_policy::same_path(&l.path, path) {
                        l.access = access.as_str().to_string();
                        touched += 1;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        store
            .update_session_links(id, |links| {
                for l in links.iter_mut() {
                    if link_policy::same_path(&l.path, path) {
                        l.access = access.as_str().to_string();
                        touched += 1;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        if touched == 0 {
            return Err(format!("{} 不是已关联的工作区", path.display()));
        }
        self.refresh();
        Ok(self.effective(id))
    }

    /// Recompute every live session's list and tell it what changed. Both the
    /// tools (through the shared handle, at once) and the model (a notice it
    /// reads on its next turn) learn about it here.
    pub fn refresh(&self) {
        let Some(store) = self.store() else { return };
        // One read of the workspace layer for the whole sweep; only the
        // session layer differs per session.
        let workspace = link_policy::normalize(self.kept(store.load_prefs().linked_roots.iter()));
        let live: Vec<(i64, Arc<LinkedRoots>, tokio::sync::mpsc::Sender<String>)> = {
            let mut guard = self.lock();
            guard.retain(|l| l.roots.strong_count() > 0);
            guard
                .iter()
                .filter_map(|l| Some((l.id, l.roots.upgrade()?, l.notices.clone())))
                .collect()
        };
        for (id, roots, notices) in live {
            let mut all = workspace.clone();
            all.extend(self.kept(store.load_session_links(id).iter()));
            let list = link_policy::normalize(all);
            let before = roots.snapshot();
            if list[..] != before[..] {
                roots.set(list.clone());
                let _ = notices.try_send(crate::session_manager::links_notice(&list));
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Live>> {
        self.live.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn store(&self) -> Option<SessionStore> {
        SessionStore::open(&self.root).ok()
    }

    /// Stored grants that still pass the policy. A path that no longer
    /// validates (hand-edited, moved into the workspace, gone) is dropped from
    /// the reachable list and said out loud once — silently honouring it is
    /// how a stale file becomes a hole.
    fn kept<'a>(&self, prefs: impl Iterator<Item = &'a LinkedRootPref>) -> Vec<LinkedRoot> {
        prefs
            .filter_map(|pref| match link_policy::validate(&pref.path, &self.root, &self.paths) {
                Ok(canon) => Some(LinkedRoot {
                    path: canon,
                    access: link_policy::access_from_label(&pref.access),
                }),
                Err(reason) => {
                    tracing::warn!(
                        path = %pref.path.display(),
                        %reason,
                        "ignoring a stored linked workspace that no longer validates"
                    );
                    None
                }
            })
            .collect()
    }
}

/// Add or update one grant by canonical path — canonical, so `/a/../b` and
/// `/b` cannot become two entries with different access.
pub fn set_link(links: &mut Vec<LinkedRootPref>, canon: &Path, access: RootAccess) {
    let label = access.as_str().to_string();
    if let Some(existing) = links.iter_mut().find(|l| link_policy::same_path(&l.path, canon)) {
        existing.access = label;
        return;
    }
    links.push(LinkedRootPref {
        path: canon.to_path_buf(),
        access: label,
    });
}

/// The hub of `root`. A free function so the manager and an actor being
/// assembled reach the same instance without passing one through every call.
pub fn hub_for(root: &Path, paths: &AppPaths) -> Arc<LinkHub> {
    LinkHub::for_workspace(root, paths)
}


