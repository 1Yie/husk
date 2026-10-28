//! Sidebar-only, single-snapshot cache. No journals, history locations, or
//! SessionMeta queue survive a list() call. The cold scan is still O(all metas).
use agent_store::{SessionMeta, SessionStore};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use husk_host::session_manager::{
    sidebar_text, ProjectSessionRow, SessionManager, SidebarProjectSource,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::{Duration, Instant},
};

const PAGE_SIZE: usize = 30;
const RECONCILE_TTL: Duration = Duration::from_secs(60);
const EXPIRED: &str = "sidebar_snapshot_expired";
static REVISION: AtomicU64 = AtomicU64::new(0);
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static PROCESS_EPOCH: LazyLock<u128> = LazyLock::new(|| {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
});
static CACHE: LazyLock<Mutex<SidebarCache>> = LazyLock::new(|| Mutex::new(SidebarCache::default()));

/// Structural ops invalidate on completion, without taking the cache mutex or
/// doing disk work on an event-forwarding thread. Failed ops invalidate too.
pub(super) struct Mutation(bool);
impl Mutation {
    pub(super) fn for_op(op: &str) -> Self {
        Self(matches!(
            op,
            "new"
                | "open"
                | "delete"
                | "fork"
                | "pin"
                | "pin_global"
                | "archive"
                | "rename"
                | "switch_workspace"
                | "pick_workspace"
                | "remove_workspace"
        ))
    }
}
impl Drop for Mutation {
    fn drop(&mut self) {
        if self.0 {
            REVISION.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[derive(Serialize)]
struct ProjectSummary {
    root: String,
    name: String,
    last_opened: u64,
    current: bool,
    office: bool,
    total: usize,
    archived_total: usize,
    running: bool,
}
#[derive(Serialize)]
struct Entry {
    root: String,
    project: String,
    current: bool,
    row: ProjectSessionRow,
}
#[derive(Serialize)]
struct Summary {
    generation: String,
    projects: Vec<ProjectSummary>,
    recents: Vec<Entry>,
    active: Option<Entry>,
}
#[derive(Serialize)]
struct Page {
    generation: String,
    root: String,
    archived: bool,
    rows: Vec<ProjectSessionRow>,
    next_cursor: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    generation: String,
    root: String,
    archived: bool,
    offset: usize,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Request {
    force: bool,
    generation: String,
    archived: bool,
    cursor: Option<String>,
    limit: Option<usize>,
    ids: Vec<i64>,
}

struct CachedProject {
    source: SidebarProjectSource,
    rows: HashMap<i64, ProjectSessionRow>,
    // Enumeration is immutable for this generation even when a live row's
    // title/preview/stamp changes. Local reorders invalidate the generation.
    order: [Vec<i64>; 2],
    dirty: bool,
}
impl CachedProject {
    fn refresh(&mut self, id: i64) -> Option<ProjectSessionRow> {
        let row = self
            .source
            .store
            .as_ref()
            .and_then(|s| s.meta(id))
            .map(|m| self.source.row(m));
        // A new/missing row or archive transition changes enumeration, unlike
        // title/preview/stamp edits. Never publish it under the old cursors.
        self.dirty |= self.rows.get(&id).map(|r| r.archived) != row.as_ref().map(|r| r.archived);
        if let Some(row) = &row {
            self.rows.insert(id, row.clone());
        } else {
            self.rows.remove(&id);
        }
        row
    }
    fn entry(&self, row: ProjectSessionRow) -> Entry {
        Entry {
            root: self.source.project.root.clone(),
            project: self.source.project.name.clone(),
            current: self.source.project.current,
            row,
        }
    }
}
#[derive(Default)]
struct SidebarCache {
    generation: String,
    revision: u64,
    built: Option<Instant>,
    projects: Vec<CachedProject>,
}
impl SidebarCache {
    fn summary(
        &mut self,
        sources: Vec<SidebarProjectSource>,
        revision: u64,
        force: bool,
        now: Instant,
        mut list: impl FnMut(&SessionStore) -> Vec<SessionMeta>,
    ) -> Summary {
        let same_roots = self.projects.len() == sources.len()
            && self
                .projects
                .iter()
                .zip(&sources)
                .all(|(a, b)| a.source.project.root == b.project.root);
        let rebuild = force
            || self
                .built
                .is_none_or(|t| now.duration_since(t) >= RECONCILE_TTL)
            || revision != self.revision
            || !same_roots
            || self.projects.iter().any(|p| p.dirty);
        if rebuild {
            self.projects = sources
                .into_iter()
                .map(|source| {
                    let mut rows: Vec<_> = source
                        .store
                        .as_ref()
                        .map(|s| list(s))
                        .unwrap_or_default()
                        .into_iter()
                        .map(|m| source.row(m))
                        .collect();
                    rows.sort_by(|a, b| {
                        b.pinned
                            .cmp(&a.pinned)
                            .then(b.updated_at.cmp(&a.updated_at))
                            .then(b.id.cmp(&a.id))
                    });
                    let order = [false, true].map(|archived| {
                        rows.iter()
                            .filter(|r| r.archived == archived)
                            .map(|r| r.id)
                            .collect()
                    });
                    CachedProject {
                        source,
                        order,
                        rows: rows.into_iter().map(|r| (r.id, r)).collect(),
                        dirty: false,
                    }
                })
                .collect();
            self.generation = format!(
                "{:x}-{}",
                *PROCESS_EPOCH,
                NEXT_GENERATION.fetch_add(1, Ordering::SeqCst)
            );
            self.revision = revision;
            self.built = Some(now);
        } else {
            for (project, source) in self.projects.iter_mut().zip(sources) {
                project.source = source;
            }
        }
        for p in &mut self.projects {
            // Clear stale flags when a parked turn ends or active id changes.
            for row in p.rows.values_mut() {
                row.active = p.source.project.current && row.id == p.source.active_id;
                row.running = p
                    .source
                    .live
                    .get(&row.id)
                    .is_some_and(|(_, running)| *running);
                if let Some((preview, _)) = p.source.live.get(&row.id) {
                    row.preview = sidebar_text(preview);
                }
            }
            if !rebuild {
                let mut ids: Vec<_> = p.source.live.keys().copied().collect();
                if p.source.active_id != 0 {
                    ids.push(p.source.active_id);
                }
                ids.sort_unstable();
                ids.dedup();
                for id in ids {
                    p.refresh(id);
                }
            }
        }
        if self.projects.iter().any(|p| p.dirty) {
            // An active/live refresh can discover an external membership edit.
            // Rebuild in this summary, once (the forced branch does no refresh),
            // instead of returning mixed counts/order or getting stuck expired.
            let sources = std::mem::take(&mut self.projects)
                .into_iter()
                .map(|p| p.source)
                .collect();
            return self.summary(sources, revision, true, now, list);
        }
        let projects = self
            .projects
            .iter()
            .map(|p| {
                let meta = &p.source.project;
                ProjectSummary {
                    root: meta.root.clone(),
                    name: meta.name.clone(),
                    last_opened: meta.last_opened,
                    current: meta.current,
                    office: meta.office,
                    running: p.source.live.values().any(|(_, running)| *running),
                    total: p.rows.values().filter(|r| !r.archived).count(),
                    archived_total: p.rows.values().filter(|r| r.archived).count(),
                }
            })
            .collect();
        let mut all: Vec<_> = self
            .projects
            .iter()
            .filter(|p| !p.source.project.office)
            .flat_map(|p| {
                p.rows
                    .values()
                    .filter(|r| !r.archived)
                    .map(|r| p.entry(r.clone()))
            })
            .collect();
        all.sort_by(|a, b| {
            b.row
                .updated_at
                .cmp(&a.row.updated_at)
                .then(a.root.cmp(&b.root))
                .then(b.row.id.cmp(&a.row.id))
        });
        let mut remaining = 6;
        all.retain(|e| {
            e.row.pinned_global
                || if remaining > 0 {
                    remaining -= 1;
                    true
                } else {
                    false
                }
        });
        all.sort_by_key(|e| !e.row.pinned_global);
        let active = self
            .projects
            .iter()
            .find(|p| p.source.project.current)
            .and_then(|p| p.rows.get(&p.source.active_id).map(|r| p.entry(r.clone())));
        Summary {
            generation: self.generation.clone(),
            projects,
            recents: all,
            active,
        }
    }

    fn valid(&self, generation: &str, revision: u64, now: Instant) -> Result<(), String> {
        if generation.is_empty()
            || generation != self.generation
            || self.revision != revision
            || self
                .built
                .is_none_or(|t| now.duration_since(t) >= RECONCILE_TTL)
            || self.projects.iter().any(|p| p.dirty)
        {
            return Err(EXPIRED.into());
        }
        Ok(())
    }
    fn page(
        &self,
        root: &str,
        archived: bool,
        generation: &str,
        cursor: Option<&str>,
        limit: usize,
        revision: u64,
        now: Instant,
    ) -> Result<Page, String> {
        self.valid(generation, revision, now)?;
        if !(1..=PAGE_SIZE).contains(&limit) {
            return Err("sidebar limit must be 1..30".into());
        }
        let p = self
            .projects
            .iter()
            .find(|p| p.source.project.root == root)
            .ok_or(EXPIRED)?;
        let ids = &p.order[usize::from(archived)];
        let offset = match cursor {
            None => 0,
            Some(token) => {
                if token.len() > 8192 {
                    return Err("invalid sidebar cursor".into());
                }
                let raw = URL_SAFE_NO_PAD
                    .decode(token)
                    .map_err(|_| "invalid sidebar cursor")?;
                let cursor: Cursor =
                    serde_json::from_slice(&raw).map_err(|_| "invalid sidebar cursor")?;
                if cursor.root != root
                    || cursor.archived != archived
                    || cursor.generation != generation
                    || cursor.offset > ids.len()
                {
                    return Err("invalid sidebar cursor".into());
                }
                cursor.offset
            }
        };
        let end = offset.saturating_add(limit).min(ids.len());
        let rows = ids[offset..end]
            .iter()
            .filter_map(|id| p.rows.get(id).filter(|r| r.archived == archived).cloned())
            .collect();
        let next_cursor = if end < ids.len() {
            Some(
                URL_SAFE_NO_PAD.encode(
                    serde_json::to_vec(&Cursor {
                        generation: generation.into(),
                        root: root.into(),
                        archived,
                        offset: end,
                    })
                    .map_err(|e| e.to_string())?,
                ),
            )
        } else {
            None
        };
        Ok(Page {
            generation: generation.into(),
            root: root.into(),
            archived,
            rows,
            next_cursor,
        })
    }
}

pub(super) fn handles(op: &str) -> bool {
    matches!(
        op,
        "sidebar" | "sidebar_page" | "sidebar_rows" | "sidebar_row" | "session_title"
    )
}

pub(super) fn dispatch(
    state: &Arc<Mutex<SessionManager>>,
    op: &str,
    root: Option<&str>,
    id: Option<i64>,
    payload: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let request: Request = serde_json::from_value(payload.unwrap_or_else(|| serde_json::json!({})))
        .map_err(|_| "invalid sidebar request")?;
    if matches!(op, "sidebar" | "sidebar_row" | "session_title") {
        let (seed, revision) = {
            let mgr = state.lock().map_err(|e| e.to_string())?;
            (mgr.sidebar_seed(), REVISION.load(Ordering::SeqCst))
        };
        // Including canonicalization, recents IO, and opening inactive stores.
        let sources = seed.snapshot().sidebar_sources();
        if op != "sidebar" {
            let root = root.ok_or("session detail needs path")?;
            let id = id.ok_or("session detail needs id")?;
            let source = sources.iter().find(|p| p.project.root == root);
            let meta = source.and_then(|p| p.store.as_ref().and_then(|s| s.meta(id)));
            // Explicit editor detail, never placed in the bounded sidebar cache.
            if op == "session_title" {
                return meta
                    .map(|m| serde_json::json!({ "title": m.title }))
                    .ok_or_else(|| "session not found".into());
            }
            let row = source.and_then(|p| meta.map(|m| p.row(m)));
            return Ok(serde_json::json!({ "known_project": source.is_some(), "row": row }));
        }
        let mut cache = CACHE.lock().map_err(|e| e.to_string())?;
        if revision != REVISION.load(Ordering::SeqCst) {
            return Err(EXPIRED.into());
        }
        let summary = cache.summary(
            sources,
            revision,
            request.force,
            Instant::now(),
            SessionStore::list,
        );
        if revision != REVISION.load(Ordering::SeqCst) {
            return Err(EXPIRED.into());
        }
        return serde_json::to_value(summary).map_err(|e| e.to_string());
    }
    let root = root.ok_or("sidebar page/rows needs path")?;
    let mut cache = CACHE.lock().map_err(|e| e.to_string())?;
    let revision = REVISION.load(Ordering::SeqCst);
    let now = Instant::now();
    cache.valid(&request.generation, revision, now)?;
    let result = if op == "sidebar_page" {
        serde_json::to_value(cache.page(
            root,
            request.archived,
            &request.generation,
            request.cursor.as_deref(),
            request.limit.unwrap_or(PAGE_SIZE),
            revision,
            now,
        )?)
        .map_err(|e| e.to_string())?
    } else {
        if request.ids.len() > PAGE_SIZE {
            return Err("sidebar rows accepts at most 30 ids".into());
        }
        let p = cache
            .projects
            .iter_mut()
            .find(|p| p.source.project.root == root)
            .ok_or(EXPIRED)?;
        let mut rows = Vec::new();
        let mut missing = Vec::new();
        for id in request.ids {
            if let Some(row) = p.refresh(id) {
                rows.push(row);
            } else {
                missing.push(id);
            }
        }
        serde_json::json!({ "generation": cache.generation, "rows": rows, "missing": missing })
    };
    cache.valid(&request.generation, revision, now)?;
    if revision != REVISION.load(Ordering::SeqCst) {
        return Err(EXPIRED.into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use husk_host::session_manager::{ProjectOverview, SidebarProjectSource};
    use std::{cell::Cell, collections::HashMap, sync::Arc, time::Instant};

    fn source(store: Arc<agent_store::SessionStore>, root: &str) -> SidebarProjectSource {
        SidebarProjectSource {
            project: ProjectOverview {
                root: root.into(),
                name: root.into(),
                last_opened: 0,
                current: root == "/a",
                office: false,
                sessions: Vec::new(),
            },
            store: Some(store),
            live: HashMap::new(),
            active_id: 1,
        }
    }

    #[test]
    fn sidebar_pages_are_bounded_stable_and_do_not_list_again() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            agent_store::SessionStore::open_at(dir.path(), &dir.path().join("sessions")).unwrap(),
        );
        for id in 1..=95 {
            store
                .upsert_meta(agent_store::SessionMeta {
                    id,
                    title: format!("row {id}"),
                    updated_at: 10,
                    pinned: id == 1,
                    pinned_global: id == 2,
                    archived: id > 70,
                    ..Default::default()
                })
                .unwrap();
        }
        let calls = Cell::new(0);
        let mut cache = SidebarCache::default();
        let now = Instant::now();
        let summary = cache.summary(
            vec![source(store.clone(), "/a"), source(store.clone(), "/b")],
            0,
            false,
            now,
            |s| {
                calls.set(calls.get() + 1);
                s.list()
            },
        );
        assert_eq!(calls.get(), 2);
        assert_eq!(summary.projects[0].total, 70);
        assert_eq!(summary.projects[0].archived_total, 25);
        assert_eq!(
            summary
                .recents
                .iter()
                .filter(|e| e.row.pinned_global)
                .count(),
            2
        );
        assert_eq!(summary.active.as_ref().unwrap().row.id, 1);
        let gen = summary.generation;
        let page = cache.page("/a", false, &gen, None, 30, 0, now).unwrap();
        assert_eq!(page.rows.len(), 30);
        assert_eq!(page.rows[0].id, 1);
        assert_eq!(page.rows[1].id, 70);
        assert!(page.rows.iter().all(|r| !r.archived));
        let second = cache
            .page("/a", false, &gen, page.next_cursor.as_deref(), 30, 0, now)
            .unwrap();
        let third = cache
            .page("/a", false, &gen, second.next_cursor.as_deref(), 30, 0, now)
            .unwrap();
        assert_eq!(third.rows.len(), 10);
        assert!(third.next_cursor.is_none());
        let ids: std::collections::HashSet<_> = page
            .rows
            .iter()
            .chain(&second.rows)
            .chain(&third.rows)
            .map(|r| r.id)
            .collect();
        assert_eq!(ids.len(), 70);
        assert!(cache
            .page("/b", false, &gen, page.next_cursor.as_deref(), 30, 0, now)
            .is_err());
        assert!(cache
            .page("/a", true, &gen, page.next_cursor.as_deref(), 30, 0, now)
            .is_err());
        assert!(cache
            .page("/a", false, &gen, Some("bad"), 30, 0, now)
            .is_err());
        assert!(cache.page("/a", false, &gen, None, 31, 0, now).is_err());
        assert!(cache.page("/a", false, &gen, None, 30, 1, now).is_err());
        assert!(cache
            .page("/a", false, &gen, None, 30, 0, now + RECONCILE_TTL)
            .is_err());
        cache.summary(
            vec![source(store.clone(), "/a"), source(store.clone(), "/b")],
            0,
            false,
            now,
            |_| panic!("cached poll must not list"),
        );
        let forced = cache.summary(
            vec![source(store.clone(), "/a"), source(store, "/b")],
            0,
            true,
            now,
            |s| {
                calls.set(calls.get() + 1);
                s.list()
            },
        );
        assert_ne!(forced.generation, gen);
        assert_eq!(calls.get(), 4);
        assert!(cache.page("/a", false, &gen, None, 30, 0, now).is_err());
    }

    #[test]
    fn sidebar_empty_and_exact_boundary_pages_have_no_next_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::open_at(dir.path(), &dir.path().join("rows")).unwrap());
        let now = Instant::now();
        let mut cache = SidebarCache::default();
        let empty = cache.summary(
            vec![source(store.clone(), "/a")],
            0,
            false,
            now,
            SessionStore::list,
        );
        assert_eq!(empty.projects[0].total, 0);
        assert!(empty.active.is_none());
        let page = cache
            .page("/a", false, &empty.generation, None, 30, 0, now)
            .unwrap();
        assert!(page.rows.is_empty() && page.next_cursor.is_none());
        assert!(cache
            .page("/a", false, &empty.generation, None, 0, 0, now)
            .is_err());
        for id in 1..=30 {
            store
                .upsert_meta(SessionMeta {
                    id,
                    updated_at: 1,
                    ..Default::default()
                })
                .unwrap();
        }
        let full = cache.summary(vec![source(store, "/a")], 0, true, now, SessionStore::list);
        let page = cache
            .page("/a", false, &full.generation, None, 30, 0, now)
            .unwrap();
        assert_eq!(page.rows.len(), 30);
        assert!(page.next_cursor.is_none());
        assert_eq!(page.rows[0].id, 30);
        assert_eq!(page.rows[29].id, 1);
        let wire = serde_json::to_value(full).unwrap();
        assert!(wire["projects"][0].get("sessions").is_none());
        let wire = serde_json::to_value(page).unwrap();
        assert!(wire.get("history").is_none());
        assert_eq!(wire["rows"].as_array().unwrap().len(), 30);
    }

    #[test]
    fn sidebar_live_active_office_and_parked_rows_survive_without_relisting() {
        let dir = tempfile::tempdir().unwrap();
        let a = Arc::new(
            SessionStore::open_at(&dir.path().join("a"), &dir.path().join("a-rows")).unwrap(),
        );
        let b = Arc::new(
            SessionStore::open_at(&dir.path().join("b"), &dir.path().join("b-rows")).unwrap(),
        );
        for id in 1..=80 {
            a.upsert_meta(SessionMeta {
                id,
                title: format!("a{id}"),
                updated_at: id as u64,
                ..Default::default()
            })
            .unwrap();
        }
        b.upsert_meta(SessionMeta {
            id: 1,
            title: "b1".into(),
            pinned_global: true,
            ..Default::default()
        })
        .unwrap();
        let sources = || {
            let mut active = source(a.clone(), "/a");
            active.live.insert(1, ("live a".into(), true));
            let mut parked = source(b.clone(), "/b");
            parked.live.insert(1, ("parked b".into(), true));
            let mut office = source(b.clone(), "/office");
            office.project.office = true;
            office.live.insert(1, ("office".into(), true));
            vec![active, parked, office]
        };
        let now = Instant::now();
        let mut cache = SidebarCache::default();
        let summary = cache.summary(sources(), 0, false, now, SessionStore::list);
        let gen = summary.generation;
        let active = summary.active.unwrap();
        assert_eq!(active.row.id, 1);
        assert_eq!(active.row.preview, "live a");
        assert!(active.row.active && active.row.running);
        assert!(!cache
            .page("/a", false, &gen, None, 30, 0, now)
            .unwrap()
            .rows
            .iter()
            .any(|r| r.id == 1));
        let parked = cache.page("/b", false, &gen, None, 30, 0, now).unwrap();
        assert_eq!(parked.rows[0].title, "b1");
        assert_eq!(parked.rows[0].preview, "parked b");
        assert!(parked.rows[0].running && !parked.rows[0].active);
        assert!(summary.projects.iter().all(|p| p.running));
        assert!(summary.recents.iter().all(|e| e.root != "/office"));
        a.upsert_meta(SessionMeta {
            id: 1,
            title: "renamed while running".into(),
            updated_at: 1000,
            ..Default::default()
        })
        .unwrap();
        let updated = cache.summary(sources(), 0, false, now, |_| {
            panic!("live poll must not list")
        });
        assert_eq!(updated.generation, gen);
        assert_eq!(updated.active.unwrap().row.title, "renamed while running");
        assert!(updated
            .recents
            .iter()
            .any(|e| e.root == "/a" && e.row.id == 1));
        let mut stopped = sources();
        for p in &mut stopped {
            p.live.clear();
        }
        let stopped = cache.summary(stopped, 0, false, now, |_| {
            panic!("stop overlay must not list")
        });
        assert!(stopped.projects.iter().all(|p| !p.running));
        assert!(!stopped.active.unwrap().row.running);
    }

    #[test]
    fn sidebar_reconciles_external_new_delete_and_rename_after_sixty_seconds() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::open_at(dir.path(), &dir.path().join("rows")).unwrap());
        store
            .upsert_meta(SessionMeta {
                id: 1,
                ..Default::default()
            })
            .unwrap();
        let now = Instant::now();
        let mut cache = SidebarCache::default();
        let first = cache.summary(
            vec![source(store.clone(), "/a")],
            0,
            false,
            now,
            SessionStore::list,
        );
        store
            .upsert_meta(SessionMeta {
                id: 9,
                title: "external".into(),
                updated_at: 1000,
                ..Default::default()
            })
            .unwrap();
        let before = cache.summary(
            vec![source(store.clone(), "/a")],
            0,
            false,
            now + Duration::from_secs(59),
            |_| panic!("TTL not reached"),
        );
        assert_eq!(before.generation, first.generation);
        assert_eq!(before.projects[0].total, 1);
        let next = cache.summary(
            vec![source(store.clone(), "/a")],
            0,
            false,
            now + RECONCILE_TTL,
            SessionStore::list,
        );
        assert_ne!(next.generation, first.generation);
        assert_eq!(next.projects[0].total, 2);
        assert_eq!(next.recents[0].row.title, "external");
        store.rename(9, "new title").unwrap();
        let refreshed = cache.projects[0].refresh(9).unwrap();
        assert_eq!(refreshed.title, "new title");
        store.remove(9).unwrap();
        assert!(cache.projects[0].refresh(9).is_none());
        let forced = cache.summary(
            vec![source(store, "/a")],
            1,
            false,
            now + RECONCILE_TTL,
            SessionStore::list,
        );
        assert_ne!(forced.generation, next.generation);
        assert_eq!(forced.projects[0].total, 1);
    }

    #[test]
    fn sidebar_external_archive_membership_expires_loaded_cursors_and_rebuilds() {
        for archived in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let rows = dir.path().join("rows");
            let store = Arc::new(SessionStore::open_at(dir.path(), &rows).unwrap());
            let external = SessionStore::open_at(dir.path(), &rows).unwrap();
            for id in 1..=65 {
                store
                    .upsert_meta(SessionMeta {
                        id,
                        archived,
                        updated_at: id as u64,
                        ..Default::default()
                    })
                    .unwrap();
            }
            let sources = || {
                let mut p = source(store.clone(), "/a");
                p.active_id = 0;
                vec![p]
            };
            let now = Instant::now();
            let mut cache = SidebarCache::default();
            let first = cache.summary(sources(), 0, false, now, SessionStore::list);
            let page = cache
                .page("/a", archived, &first.generation, None, 30, 0, now)
                .unwrap();
            assert!(external.set_archived(65, !archived).unwrap());
            // This is the same single-meta refresh used by sidebar_rows.
            cache.projects[0].refresh(65).unwrap();
            assert_eq!(
                cache
                    .page(
                        "/a",
                        archived,
                        &first.generation,
                        page.next_cursor.as_deref(),
                        30,
                        0,
                        now
                    )
                    .err()
                    .as_deref(),
                Some(EXPIRED)
            );
            let next = cache.summary(sources(), 0, false, now, SessionStore::list);
            assert_ne!(next.generation, first.generation);
            let moved = cache
                .page("/a", !archived, &next.generation, None, 30, 0, now)
                .unwrap();
            assert_eq!(moved.rows.iter().map(|r| r.id).collect::<Vec<_>>(), [65]);
            assert_eq!(
                cache
                    .page("/a", archived, &first.generation, None, 30, 0, now)
                    .err()
                    .as_deref(),
                Some(EXPIRED)
            );
            // Ordinary edits update the row without rescanning or reordering.
            external.rename(65, "external title").unwrap();
            assert_eq!(
                cache.projects[0].refresh(65).unwrap().title,
                "external title"
            );
            let renamed = cache.summary(sources(), 0, false, now, |_| {
                panic!("title-only refresh must not list")
            });
            assert_eq!(renamed.generation, next.generation);
        }
    }

    #[test]
    fn sidebar_external_active_archive_new_and_delete_membership_rebuild_on_poll() {
        let dir = tempfile::tempdir().unwrap();
        let rows = dir.path().join("rows");
        let store = Arc::new(SessionStore::open_at(dir.path(), &rows).unwrap());
        let external = SessionStore::open_at(dir.path(), &rows).unwrap();
        store
            .upsert_meta(SessionMeta {
                id: 1,
                ..Default::default()
            })
            .unwrap();
        let now = Instant::now();
        let mut cache = SidebarCache::default();
        let mut generation = cache
            .summary(
                vec![source(store.clone(), "/a")],
                0,
                false,
                now,
                SessionStore::list,
            )
            .generation;
        for archived in [true, false] {
            external.set_archived(1, archived).unwrap();
            let next = cache.summary(
                vec![source(store.clone(), "/a")],
                0,
                false,
                now,
                SessionStore::list,
            );
            assert_ne!(
                next.generation, generation,
                "active refresh must not retain a stale enumeration"
            );
            assert_eq!(next.active.unwrap().row.archived, archived);
            assert_eq!(
                cache
                    .page("/a", archived, &next.generation, None, 30, 0, now)
                    .unwrap()
                    .rows[0]
                    .id,
                1
            );
            assert_eq!(
                cache.valid(&generation, 0, now).err().as_deref(),
                Some(EXPIRED)
            );
            generation = next.generation;
        }
        external.remove(1).unwrap();
        let empty = cache.summary(
            vec![source(store.clone(), "/a")],
            0,
            false,
            now,
            SessionStore::list,
        );
        assert_ne!(empty.generation, generation);
        assert!(empty.active.is_none());
        let sources = || {
            let mut p = source(store.clone(), "/a");
            p.active_id = 9;
            vec![p]
        };
        let missing = cache.summary(sources(), 0, false, now, |_| {
            panic!("unknown active id is not a membership change")
        });
        assert_eq!(missing.generation, empty.generation);
        assert!(missing.active.is_none());
        external
            .upsert_meta(SessionMeta {
                id: 9,
                title: "new external row".into(),
                ..Default::default()
            })
            .unwrap();
        let added = cache.summary(sources(), 0, false, now, SessionStore::list);
        assert_ne!(added.generation, empty.generation);
        assert_eq!(added.projects[0].total, 1);
        assert_eq!(
            cache
                .page("/a", false, &added.generation, None, 30, 0, now)
                .unwrap()
                .rows[0]
                .title,
            "new external row"
        );
    }

    #[test]
    fn sidebar_text_projection_bounds_cache_pages_rows_summary_and_live_overlays() {
        for text in [
            "English ".repeat(200),
            "中文".repeat(700),
            "🙂🦀".repeat(700),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let store =
                Arc::new(SessionStore::open_at(dir.path(), &dir.path().join("rows")).unwrap());
            for id in 1..=3 {
                store
                    .upsert_meta(SessionMeta {
                        id,
                        title: text.clone(),
                        preview: text.clone(),
                        pinned_global: id == 1,
                        archived: id == 3,
                        ..Default::default()
                    })
                    .unwrap();
            }
            let sources = || {
                let mut p = source(store.clone(), "/a");
                p.live.insert(1, (text.clone(), true));
                p.live.insert(2, (text.clone(), true));
                vec![p]
            };
            let check = |r: &ProjectSessionRow| {
                let expected: String = text.chars().take(512).collect();
                assert_eq!(r.title, expected);
                assert_eq!(r.preview, expected);
            };
            // sidebar_row uses this projection directly, independent of CACHE.
            check(&sources()[0].row(store.meta(1).unwrap()));
            let now = Instant::now();
            let mut cache = SidebarCache::default();
            let first = cache.summary(sources(), 0, false, now, SessionStore::list);
            check(&first.active.unwrap().row);
            for entry in &first.recents {
                check(&entry.row);
            }
            for archived in [false, true] {
                for row in cache
                    .page("/a", archived, &first.generation, None, 30, 0, now)
                    .unwrap()
                    .rows
                {
                    check(&row);
                }
            }
            // sidebar_rows and both cold/warm direct live overlays share the cap.
            check(&cache.projects[0].refresh(2).unwrap());
            let poll = cache.summary(sources(), 0, false, now, |_| {
                panic!("text refresh must not list")
            });
            check(&poll.active.unwrap().row);
            for row in cache.projects[0].rows.values() {
                check(row);
            }
            assert_eq!(store.meta(1).unwrap().title, text);
            assert_eq!(store.list()[0].preview, text);
        }
    }

    #[test]
    fn sidebar_request_validation_rejects_malformed_and_oversized_page_inputs() {
        for payload in [
            serde_json::json!({"limit": -1}),
            serde_json::json!({"archived": "yes"}),
            serde_json::json!({"cursor": {"offset": 30}}),
            serde_json::json!({"unknown": true}),
        ] {
            assert!(serde_json::from_value::<Request>(payload).is_err());
        }
        let before = REVISION.load(Ordering::SeqCst);
        for op in [
            "new",
            "open",
            "delete",
            "fork",
            "pin",
            "pin_global",
            "archive",
            "rename",
            "switch_workspace",
            "pick_workspace",
            "remove_workspace",
        ] {
            drop(Mutation::for_op(op));
        }
        assert_eq!(REVISION.load(Ordering::SeqCst), before + 11);
    }
}
