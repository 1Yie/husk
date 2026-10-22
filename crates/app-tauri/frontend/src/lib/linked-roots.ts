// Linked workspaces — the directories a session may reach beyond its own
// root. One hook for the composer's grant menu, so the list, the mutations and
// the optimistic updates have exactly one owner.

import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import * as agent from "@/lib/agent-ipc/index";
import type { LinkedRoot, LinkedRootLayers } from "@/lib/agent-ipc/index";

const EMPTY: LinkedRootLayers = { workspace: [], session: [] };

/** The last path segment — what a row is called before the full path. */
export function linkedName(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts[parts.length - 1] || path;
}

export function useLinkedRoots(root?: string, id?: number | null) {
  const [layers, setLayers] = useState<LinkedRootLayers>(EMPTY);
  const [busy, setBusy] = useState(false);
  const ready = Boolean(root) && typeof id === "number";

  const reload = useCallback(async () => {
    if (!root || typeof id !== "number") return;
    try {
      setLayers((await agent.listLinkedRoots(root, id)) ?? EMPTY);
    } catch (e) {
      // A session with no store yet has no grants; the next call fills it in.
      console.error("listLinkedRoots failed:", e);
    }
  }, [root, id]);

  useEffect(() => {
    void reload();
  }, [reload]);

  /** Run one mutation. The reply is authoritative — but the row is updated in
   *  place first (see `patchAccess`), so the list never re-renders from a
   *  half-written state while the round trip is out. */
  const run = useCallback(
    async (call: () => Promise<LinkedRootLayers>) => {
      if (busy) return;
      setBusy(true);
      try {
        setLayers((await call()) ?? EMPTY);
      } catch (e) {
        toast.error(typeof e === "string" && e ? e : "关联失败");
        void reload();
      } finally {
        setBusy(false);
      }
    },
    [busy, reload],
  );

  /** Flip one row's access in place. Re-listing under the cursor made the row
   *  (and the row beneath it) blink for a frame; a toggled grant is a local
   *  fact until the store says otherwise. */
  const patchAccess = useCallback((path: string, access: LinkedRoot["access"]) => {
    const rewrite = (rows: LinkedRoot[]) =>
      rows.map((r) => (r.path === path ? { ...r, access } : r));
    setLayers((prev) => ({
      workspace: rewrite(prev.workspace),
      session: rewrite(prev.session),
    }));
  }, []);

  /** Drop one row in place, same reason. */
  const dropRow = useCallback((path: string) => {
    const without = (rows: LinkedRoot[]) => rows.filter((r) => r.path !== path);
    setLayers((prev) => ({
      workspace: without(prev.workspace),
      session: without(prev.session),
    }));
  }, []);

  const add = useCallback(
    async (path: string, access: LinkedRoot["access"], persist: boolean) => {
      if (!root || typeof id !== "number") return;
      await run(() => agent.linkRoot(root, id, path, access, persist));
    },
    [root, id, run],
  );

  /** Pick a folder, then grant it. The dialog runs before the grant, so a
   *  cancelled pick changes nothing. */
  const pickAndAdd = useCallback(
    async (access: LinkedRoot["access"], persist: boolean) => {
      if (!root || typeof id !== "number" || busy) return;
      const picked = await agent.pickLinkDir().catch(() => null);
      if (picked) await add(picked, access, persist);
    },
    [root, id, busy, add],
  );

  const remove = useCallback(
    async (path: string) => {
      if (!root || typeof id !== "number" || busy) return;
      dropRow(path);
      await run(() => agent.unlinkRoot(root, id, path));
    },
    [root, id, busy, run, dropRow],
  );

  const setAccess = useCallback(
    async (path: string, access: LinkedRoot["access"]) => {
      if (!root || typeof id !== "number" || busy) return;
      patchAccess(path, access);
      await run(() => agent.setLinkAccess(root, id, path, access));
    },
    [root, id, busy, run, patchAccess],
  );

  /** Both layers flattened, workspace first — the order the prompt lists. A
   *  path granted both ways appears once (the broader grant wins): two rows
   *  with one identity would collide on their React key. */
  const all: LinkedRoot[] = (() => {
    const seen = new Set<string>();
    const out: LinkedRoot[] = [];
    for (const r of [...layers.workspace, ...layers.session]) {
      if (seen.has(r.path)) continue;
      seen.add(r.path);
      out.push(r);
    }
    return out;
  })();

  return { workspace: layers.workspace, session: layers.session, all, ready, busy, reload, add, pickAndAdd, remove, setAccess };
}
