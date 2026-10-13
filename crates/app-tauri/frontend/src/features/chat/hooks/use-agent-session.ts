// useAgentSession — the project tree and session-list commands for the
// sidebar. Polls the backend on a 2s cadence; structural ops (new/open/
// delete/fork) refresh through `refresh`.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as agent from "@/lib/agent-ipc/index";
import type { ProjectOverview } from "@/types";

export function useAgentSession() {
  const [projects, setProjects] = useState<ProjectOverview[]>([]);
  /** First `listProjects` resolved — until then `sessions === []` means
   *  "still loading", not "workspace has no sessions". */
  const [loaded, setLoaded] = useState(false);

  // One `listProjects` in flight at a time. The backend re-reads every
  // recent project's session index per call, so a poll that lands while the
  // previous one is still running only queues behind it and holds the kernel
  // lock longer. Structural refreshes (`force`) still go out regardless.
  const inFlight = useRef<Promise<ProjectOverview[]> | null>(null);

  // Fetch only — no state write. A caller that must land the tree in the
  // same commit as other state (workspace switch: the sidebar and the chat
  // view appear together) takes the rows and commits them with `commit`.
  const fetchProjects = useCallback(async (): Promise<ProjectOverview[] | null> => {
    try {
      return await agent.listProjects();
    } catch {
      // The 2s poll races backend ops (a switch holds the kernel lock) —
      // keep the last good tree instead of surfacing an unhandled rejection.
      return null;
    }
  }, []);

  const commit = useCallback((rows: ProjectOverview[]) => {
    setProjects(rows);
    setLoaded(true);
  }, []);

  const refresh = useCallback(async ({ force = false } = {}) => {
    if (!force && inFlight.current) return inFlight.current;
    const request = (async () => {
      const rows = await fetchProjects();
      if (rows) commit(rows);
      // A failed first load must still end the skeleton — otherwise the
      // sidebar would draw placeholders forever. The tree stays empty and the
      // ordinary empty-state text takes over.
      else setLoaded(true);
      inFlight.current = null;
      return rows ?? [];
    })();
    inFlight.current = request;
    return request;
  }, [fetchProjects, commit]);

  const newSession = useCallback(async () => {
    const newId = await agent.newSession();
    await refresh({ force: true });
    return newId;
  }, [refresh]);

  const openSession = useCallback(
    async (id: number) => {
      const r = await agent.openSession(id);
      await refresh({ force: true });
      return r;
    },
    [refresh]
  );

  useEffect(() => {
    void refresh();
    const t = setInterval(() => void refresh(), 2000);
    return () => clearInterval(t);
  }, [refresh]);

  // The active workspace's rows — boot resume, the active title and the
  // delete dialog all derive from the project tree, which reads the store
  // fresh instead of the manager's structural-op metas.
  const sessions = useMemo(
    () => projects.find((p) => p.current)?.sessions ?? [],
    [projects]
  );

  return { projects, sessions, loaded, refresh, fetchProjects, commit, newSession, openSession };
}
