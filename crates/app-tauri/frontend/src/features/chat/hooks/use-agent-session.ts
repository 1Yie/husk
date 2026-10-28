// Sidebar summary + independently paged project/archive lists. Polling only
// refreshes live/previously loaded rows; it never walks unread pages.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as agent from "@/lib/agent-ipc/index";
import type { ProjectOverview, ProjectSessionRow, SidebarSummary } from "@/types";

export interface SidebarPageState {
  rows: ProjectSessionRow[];
  generation: string;
  cursor: string | null;
  loaded: boolean;
  loading: boolean;
  pages: number;
  error?: string;
}
const emptyPage = (): SidebarPageState => ({ rows: [], generation: "", cursor: null, loaded: false, loading: false, pages: 0 });
const keyFor = (root: string, archived: boolean) => JSON.stringify([root, archived]);
const unique = (rows: ProjectSessionRow[]) => [...new Map(rows.map((r) => [r.id, r])).values()];
const emptySummary: SidebarSummary = { generation: "", projects: [], recents: [], active: null };

type PageFailure = {
  phase: "page" | "refresh";
  error: string;
  expired: boolean;
  automaticAttempted: boolean;
  summaryRevision: number;
  pendingPageError?: string;
};

const snapshotExpired = (error: unknown) => String(error).includes("sidebar_snapshot_expired");

export function useAgentSession() {
  const summary = useRef<SidebarSummary>(emptySummary);
  const buckets = useRef(new Map<string, SidebarPageState>());
  const failures = useRef(new Map<string, PageFailure>());
  const visiblePages = useRef(new Map<string, Set<symbol>>());
  const summaryRevision = useRef(0);
  const summaryRecoveryUsed = useRef(false);
  const epoch = useRef(0);
  const mounted = useRef(true);
  const summaryFlight = useRef<Promise<ProjectOverview[]> | null>(null);
  const pageFlights = useRef(new Map<string, Promise<void>>());
  const [version, setVersion] = useState(0);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string>();
  const summaryError = useRef<string>();
  const publish = useCallback(() => { if (mounted.current) setVersion((v) => v + 1); }, []);

  /** Invalidate ownership, not backend IO. Old completions cannot release a
   * newer request's flight or change its recovery budget. */
  const invalidate = useCallback(() => {
    epoch.current++;
    summaryFlight.current = null;
    pageFlights.current.clear();
    for (const [key, page] of buckets.current) buckets.current.set(key, { ...page, loading: false });
  }, []);

  const projectRows = useCallback(() => summary.current.projects.map((p) => {
    const rows = unique([...(buckets.current.get(keyFor(p.root, false))?.rows ?? []),
      ...(buckets.current.get(keyFor(p.root, true))?.rows ?? [])]);
    const active = summary.current.active;
    if (active?.root === p.root) {
      const index = rows.findIndex((r) => r.id === active.row.id);
      if (index < 0) rows.unshift(active.row);
      else rows[index] = active.row;
    }
    return { ...p, sessions: rows.map((r) => ({ ...r, active: active?.root === p.root && active.row.id === r.id })) };
  }), []);

  const loadPage = useCallback((root: string, archived = false, recovery = false): Promise<void> => {
    const key = keyFor(root, archived);
    const inFlight = pageFlights.current.get(key);
    if (inFlight) return inFlight;
    const previous = buckets.current.get(key) ?? emptyPage();
    if (!summary.current.generation || summaryFlight.current ||
      !summary.current.projects.some((p) => p.root === root) ||
      (previous.error && !recovery) || (previous.loaded && !previous.cursor)) return Promise.resolve();
    const failure = failures.current.get(key);
    if (recovery && (!failure?.expired || failure.automaticAttempted)) return Promise.resolve();
    if (recovery && failure) failure.automaticAttempted = true;
    const ticket = epoch.current;
    const generation = summary.current.generation;
    const current = () => mounted.current && epoch.current === ticket && summary.current.generation === generation;
    // Register ownership before invoking IPC: a synchronous throw must not
    // run finally before the request has entered the flight map.
    let complete!: () => void;
    const request = new Promise<void>((resolve) => { complete = resolve; });
    pageFlights.current.set(key, request);
    buckets.current.set(key, { ...previous, loading: true, error: undefined, generation });
    publish();
    void (async () => {
      try {
        const page = await agent.sidebarPage(root, archived, generation, previous.cursor);
        if (!current()) return;
        if (page.generation !== generation || page.root !== root || page.archived !== archived) throw new Error("sidebar_snapshot_expired");
        buckets.current.set(key, { rows: unique([...previous.rows, ...page.rows]), generation,
          cursor: page.next_cursor, loaded: true, loading: false, pages: previous.pages + 1 });
        failures.current.delete(key);
      } catch (e) {
        if (!current()) return;
        failures.current.set(key, {
          phase: "page", error: String(e), expired: snapshotExpired(e),
          automaticAttempted: recovery || Boolean(failure?.automaticAttempted),
          summaryRevision: summaryRevision.current,
        });
        buckets.current.set(key, { ...previous, generation, loading: false, error: String(e) });
      } finally {
        if (pageFlights.current.get(key) === request) {
          pageFlights.current.delete(key);
          if (current()) publish();
        }
        complete();
      }
    })();
    return request;
  }, [publish]);

  const loadMore = useCallback((root: string, archived = false) => loadPage(root, archived), [loadPage]);

  const resumeVisiblePages = useCallback(() => {
    if (summaryFlight.current || summaryError.current) return;
    for (const key of visiblePages.current.keys()) {
      const page = buckets.current.get(key);
      if (page?.loaded || page?.loading) continue;
      const [root, archived] = JSON.parse(key) as [string, boolean];
      if (!page?.error) {
        void loadPage(root, archived);
        continue;
      }
      const failure = failures.current.get(key);
      if (failure?.phase === "page" && failure.expired && !failure.automaticAttempted
        && summaryRevision.current > failure.summaryRevision) void loadPage(root, archived, true);
    }
  }, [loadPage]);

  const refresh = useCallback(({ force = false } = {}): Promise<ProjectOverview[]> => {
    if (!force && summaryFlight.current) return summaryFlight.current;
    if (!force && pageFlights.current.size > 0) return Promise.resolve(projectRows());
    if (!force && summaryError.current) {
      if (!snapshotExpired(summaryError.current) || summaryRecoveryUsed.current) return Promise.resolve(projectRows());
      summaryRecoveryUsed.current = true;
    }
    if (force) {
      invalidate();
      failures.current.clear();
      summaryRecoveryUsed.current = false;
    }
    const ticket = epoch.current;
    const current = () => mounted.current && epoch.current === ticket;
    let complete!: (rows: ProjectOverview[]) => void;
    const request = new Promise<ProjectOverview[]>((resolve) => { complete = resolve; });
    summaryFlight.current = request;
    void (async () => {
      let result: ProjectOverview[] = [];
      try {
        const next = await agent.sidebarSummary(force);
        if (!current()) return;
        summary.current = next;
        summaryRevision.current++;
        summaryError.current = undefined;
        summaryRecoveryUsed.current = false;
        setError(undefined);
        setLoaded(true);
        const roots = new Set(next.projects.map((p) => p.root));
        for (const [key, previous] of buckets.current) {
          const [root, archived] = JSON.parse(key) as [string, boolean];
          if (!roots.has(root)) {
            buckets.current.delete(key);
            failures.current.delete(key);
            continue;
          }
          const failure = failures.current.get(key);
          if (previous.pages === 0) {
            // First-page failures recover only while their list is mounted.
            if (force || !previous.error) buckets.current.set(key, {
              ...previous, generation: next.generation, loading: false, error: undefined,
            });
            continue;
          }
          if (!force && failure) {
            if (failure.expired) {
              if (failure.automaticAttempted) continue;
              failure.automaticAttempted = true;
            } else if (failure.phase === "refresh") continue;
          }
          const pendingPageError = failure?.pendingPageError
            ?? (failure?.phase === "page" && !failure.expired ? failure.error : undefined);
          // Keep the old cursor AND its generation until the complete rebase
          // succeeds. A failed prefix must never relabel an old cursor.
          const loading = force || Boolean(failure) || previous.generation !== next.generation;
          buckets.current.set(key, { ...previous, loading, error: pendingPageError });
          publish();
          try {
            const retained: ProjectSessionRow[] = [];
            for (let i = 0; i < previous.rows.length; i += 30) {
              const updates = await agent.sidebarRows(root, next.generation, previous.rows.slice(i, i + 30).map((r) => r.id));
              if (!current()) return;
              if (updates.generation !== next.generation) throw new Error("sidebar_snapshot_expired");
              retained.push(...updates.rows.filter((r) => Boolean(r.archived) === archived));
            }
            let rows = retained;
            let cursor = previous.cursor;
            let pages = previous.pages;
            if (previous.generation !== next.generation) {
              const prefix: ProjectSessionRow[] = [];
              cursor = null; pages = 0;
              for (let i = 0; i < previous.pages; i++) {
                const page = await agent.sidebarPage(root, archived, next.generation, cursor);
                if (!current()) return;
                if (page.generation !== next.generation || page.root !== root || page.archived !== archived) throw new Error("sidebar_snapshot_expired");
                prefix.push(...page.rows);
                cursor = page.next_cursor; pages++;
                if (!cursor) break;
              }
              rows = unique([...prefix, ...retained]);
            }
            buckets.current.set(key, { rows, cursor, pages, generation: next.generation,
              loaded: true, loading: false, error: pendingPageError });
            if (pendingPageError) failures.current.set(key, {
              phase: "page", error: pendingPageError, expired: false, automaticAttempted: false,
              summaryRevision: summaryRevision.current,
            });
            else failures.current.delete(key);
          } catch (e) {
            if (!current()) return;
            failures.current.set(key, {
              phase: "refresh", error: String(e), expired: snapshotExpired(e),
              automaticAttempted: Boolean(failure?.automaticAttempted),
              summaryRevision: summaryRevision.current, pendingPageError,
            });
            buckets.current.set(key, { ...previous, loading: false, error: String(e) });
          }
        }
        result = projectRows();
      } catch (e) {
        if (current()) {
          summaryError.current = String(e);
          setError(String(e));
          setLoaded(true);
        }
      } finally {
        if (current() && summaryFlight.current === request) {
          summaryFlight.current = null;
          publish();
          resumeVisiblePages();
        }
        complete(result);
      }
    })();
    return request;
  }, [invalidate, projectRows, publish, resumeVisiblePages]);

  const ensurePage = useCallback((root: string, archived = false) => {
    const key = keyFor(root, archived);
    const token = Symbol();
    const viewers = visiblePages.current.get(key) ?? new Set<symbol>();
    viewers.add(token);
    visiblePages.current.set(key, viewers);
    resumeVisiblePages();
    return () => {
      viewers.delete(token);
      if (!viewers.size && visiblePages.current.get(key) === viewers) visiblePages.current.delete(key);
    };
  }, [resumeVisiblePages]);

  const retryPage = useCallback(async (root: string, archived = false) => {
    const key = keyFor(root, archived);
    const wasEmpty = !buckets.current.get(key)?.pages;
    const failure = failures.current.get(key);
    await refresh({ force: true });
    const page = buckets.current.get(key);
    if (page?.error || summaryError.current) return;
    // A row-refresh failure retries its read budget, not an unread next page.
    // A mounted first page may already have resumed in refresh's finally.
    if (wasEmpty ? !page?.pages : failure?.phase === "page") await loadMore(root, archived);
  }, [refresh, loadMore]);

  const newSession = useCallback(async () => {
    invalidate();
    const id = await agent.newSession();
    const rows = await refresh({ force: true });
    return { id, rows };
  }, [invalidate, refresh]);
  const openSession = useCallback(async (id: number) => {
    invalidate();
    const result = await agent.openSession(id);
    invalidate();
    await refresh({ force: Boolean(summaryError.current) });
    return result;
  }, [invalidate, refresh]);

  useEffect(() => {
    mounted.current = true;
    void refresh();
    const timer = setInterval(() => void refresh(), 2000);
    return () => {
      mounted.current = false;
      invalidate();
      visiblePages.current.clear();
      clearInterval(timer);
    };
  }, [refresh, invalidate]);

  const projects = useMemo(projectRows, [projectRows, version]);
  const sessions = useMemo(() => {
    const project = projects.find((p) => p.current);
    if (!project) return [];
    return unique([...summary.current.recents.filter((e) => e.root === project.root).map((e) => e.row), ...project.sessions]);
  }, [projects]);
  const paging = useMemo(() => ({
    generation: summary.current.generation,
    get: (root: string, archived = false) => buckets.current.get(keyFor(root, archived)) ?? emptyPage(),
    ensurePage, loadMore, retryPage,
  }), [version, ensurePage, loadMore, retryPage]);
  return { projects, sessions, loaded, error, refresh, invalidate, newSession, openSession,
    recents: summary.current.recents, active: summary.current.active, paging };
}

export type SidebarPaging = ReturnType<typeof useAgentSession>["paging"];
