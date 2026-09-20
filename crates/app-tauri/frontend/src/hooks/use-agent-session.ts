// useAgentSession — the project tree and session-list commands for the
// sidebar. Polls the backend on a 2s cadence; structural ops (new/open/
// delete/fork) refresh through `refresh`.

import { useCallback, useEffect, useMemo, useState } from "react";
import * as agent from "../invoke/agent";
import type { ProjectOverview } from "../types";

export function useAgentSession() {
  const [projects, setProjects] = useState<ProjectOverview[]>([]);

  const refresh = useCallback(async () => {
    try {
      const rows = await agent.listProjects();
      setProjects(rows);
      return rows;
    } catch {
      // The 2s poll races backend ops (a switch holds the kernel lock) —
      // keep the last good tree instead of surfacing an unhandled rejection.
      return [];
    }
  }, []);

  const newSession = useCallback(async () => {
    const newId = await agent.newSession();
    await refresh();
    return newId;
  }, [refresh]);

  const openSession = useCallback(
    async (id: number) => {
      const r = await agent.openSession(id);
      await refresh();
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

  return { projects, sessions, refresh, newSession, openSession };
}
