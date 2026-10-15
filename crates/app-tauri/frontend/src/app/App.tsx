import { startTransition, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Toaster } from "@/components/ui/sonner";
import { toast } from "sonner";
import { activeViewNow, refreshAgentChips, useAgentStore, useRunningKeys } from "@/stores/agent-store";
import { useAgentSession } from "@/features/chat/hooks/use-agent-session";
import { viewFromHistory, viewFromHistoryChunked } from "@/features/chat/hooks/view-from-history";
import { loadReset, loadStamp } from "@/lib/load-probe";
import { getWorkspaceInfo, pickWorkspace, switchWorkspace, removeWorkspace, forkSession, deleteSession, pinSession, pinSessionGlobal, archiveSession, renameSession, exportSession, historyPage, historyRaw, type WorkspaceInfo } from "@/lib/agent-ipc/index";
import type { ProjectOverview } from "@/types";
import type { SessionRow } from "@/types";
import { SessionSidebar, type SessionActions } from "@/features/chat/components/session-sidebar/index";
import { MainLayout } from "@/app/layout/main-layout/index";
import { ChatPage } from "@/features/chat/pages/chat/index";
import { SettingsPage } from "@/features/settings/pages/index";
import { RawHistoryDialog } from "@/features/chat/components/raw-history-dialog/index";
import { WorkspaceLoadingVeil } from "@/features/chat/components/workspace-loading-veil/index";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

/** Whether `e.target` is a native-editing surface that keeps its browser
 * context menu (cut/copy/paste/select-all). Text areas and inputs rely on
 * it — Radix menus can't replicate IME/spellcheck entries there. */
function isEditableTarget(e: Event): boolean {
  const el = e.target as HTMLElement | null;
  if (!el || typeof el.closest !== "function") return false;
  return Boolean(el.closest("input, textarea, [contenteditable=true], [contenteditable='']"));
}

/** Global chrome policy for the packaged shell:
 *  - right-click belongs to app menus (sidebar rows, chat stream); the
 *    native webview menu is suppressed everywhere except editable fields
 *  - devtools shortcuts are dead keys in production builds UNLESS the
 *    user opted in via `~/.config/husk/settings.toml` `devtools = true`
 *    (read once into `window.__huskDevToolsEnabled` by `frame-probe.ts`'s
 *    sibling bootstrap in `main.tsx`).
 * Dev (`vite dev`) keeps both for debugging. */
function useAppChromeGuards() {
  useEffect(() => {
    const onContextMenu = (e: MouseEvent) => {
      // Dev keeps the native webview menu — right-click inspect is the
      // debugger's front door on WebKitGTK.
      if (import.meta.env.DEV) return;
      // Radix ContextMenuTrigger calls preventDefault itself; letting the
      // event through to a trigger is what opens our custom menu.
      // Order matters: our own triggers win even on editable surfaces (the
      // composer textarea is one), so right-clicking the input opens the app
      // menu instead of doubling up with the native one.
      const el = e.target as HTMLElement | null;
      if (el?.closest?.("[data-allow-contextmenu]")) return;
      if (isEditableTarget(e)) return;
      // Radix context-menu triggers are inside our own menu surfaces —
      // the trigger fires the React handler, then this capture-phase
      // listener would still see the event. Distinguish by what the
      // trigger rendered: a `data-state` attribute exists on the trigger
      // element once Radix wires it, but simplest reliable check is our
      // own opt-in marker applied by ContextMenuTrigger's wrapper.
      e.preventDefault();
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (import.meta.env.DEV) return;
      const k = e.key;
      const mod = e.ctrlKey || e.metaKey;
      // F12 · Ctrl/Cmd+Shift+I/J/C/K · Cmd/Ctrl+Alt+I/J/C (mac) · Ctrl+U.
      // Opt-in via `window.__huskDevToolsEnabled` (set by the dev-config
      // bootstrap in main.tsx) lets a packaged build open devtools for
      // debugging without rebuilding.
      const devtoolsEnabled =
        (window as { __huskDevToolsEnabled?: boolean }).__huskDevToolsEnabled === true;
      const devtools =
        k === "F12" ||
        (mod && e.shiftKey && ["i", "I", "j", "J", "c", "C", "k", "K"].includes(k)) ||
        (mod && e.altKey && ["i", "I", "j", "J", "c", "C"].includes(k)) ||
        (e.ctrlKey && (k === "u" || k === "U"));
      if (devtools && !devtoolsEnabled) e.preventDefault();
    };
    document.addEventListener("contextmenu", onContextMenu, true);
    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("contextmenu", onContextMenu, true);
      document.removeEventListener("keydown", onKeyDown, true);
    };
  }, []);
}

export function App() {
  useAppChromeGuards();
  const [workspace, setWorkspace] = useState<WorkspaceInfo>({ root: "", name: "", recents: [] });
  useEffect(() => {
    if (workspace.storage_warning) {
      toast.warning("当前会话不会保存到磁盘", {
        id: "session-storage-warning",
        description: workspace.storage_warning,
        duration: Infinity,
        closeButton: true,
      });
    }
  }, [workspace.storage_warning]);
  // One selector per field: the actions are stable references, so only a real
  // change in `activeId` re-renders this shell — and `useRunningKeys` keeps its
  // identity while the same sessions run, so a streaming turn never does.
  const activeId = useAgentStore((s) => s.activeId);
  const setActiveId = useAgentStore((s) => s.setActiveId);
  const loadView = useAgentStore((s) => s.loadView);
  const prependItems = useAgentStore((s) => s.prependItems);
  const dropView = useAgentStore((s) => s.dropView);
  const runningKeys = useRunningKeys();

  // The store keys every view by `root:id`, and it is the only place the active
  // view is resolved from (`useActiveView`, `activeViewNow`). Keeping its root
  // in sync with the workspace this shell is showing is therefore mandatory —
  // without it every lookup misses and the stream renders empty.
  useLayoutEffect(() => {
    useAgentStore.getState().setWorkspaceRoot(workspace.root);
  }, [workspace.root]);

  // `handleLoadOlder` and the rail's placeholder seek loop keep running through
  // closures captured at an earlier render; reading `historyStart` off a
  // stale view re-fetches the SAME page and duplicates it (→ duplicate `hi`
  // → several black marks in the rail). The store is read AT CALL TIME — the
  // ref this used to need is exactly what `activeViewNow()` returns.
  const { projects, sessions, loaded: sessionsLoaded, refresh, fetchProjects, commit, newSession, openSession } = useAgentSession();
  // True while a session/workspace switch is fetching history + rebuilding
  // the view — the stream renders a skeleton instead of a stale/empty pane.
  const [viewLoading, setViewLoading] = useState(false);
  // The full-layout switch veil — only workspace/session switches raise it.
  // Delete, fork and boot keep their in-pane skeleton (boot already has the
  // home splash), so a small action never blanks the whole window.
  const [switchVeil, setSwitchVeil] = useState(false);
  // Drop the loading veil only AFTER the mounted tree has committed and
  // painted — session loads full-mount every turn in one synchronous
  // commit, and if the veil lifts inside that same commit the app shows
  // a frozen half-frame instead of "loading → ready". Two rAFs carries
  // the flag past the next paint boundary.
  const releaseLoading = useCallback(() => {
    // Every caller reaches here after its backend call has settled, so the
    // backend's ACTIVE session is final — the first moment the header chips
    // (the active model's window and price) can be read for it. `setActiveId`
    // flips the frontend pointer BEFORE that IPC, so the refresh it triggers
    // reads the PREVIOUS session's model, and nothing re-read afterwards: the
    // header kept those numbers until the next turn. Failed flows land here
    // too; re-reading whatever is active then is still correct.
    refreshAgentChips();
    requestAnimationFrame(() =>
      // The reveal commit mounts the real turn tree (Streamdown/Streamdown
      // blocks) —
      // a transition keeps its render interruptible so the page never
      // goes dead to clicks while a long session opens.
      requestAnimationFrame(() =>
        startTransition(() => {
          setViewLoading(false);
          setSwitchVeil(false);
        }),
      ),
    );
  }, []);

  /** Fold the returned page + mount it, then lift the veil after the
   * commit has painted. */
  const mountLoadedView = useCallback(
    async (
      root: string,
      id: number,
      r: {
        history: Parameters<typeof viewFromHistory>[0];
        history_total?: number;
        turn_total?: number;
        turn_offset?: number;
        usage?: Parameters<typeof viewFromHistory>[1];
        queued_prompts?: string[];
      },
      // Runs in the same synchronous block as `loadView`, so state it sets
      // lands in the same commit as the chat view (see handleSwitchWorkspace).
      alsoCommit?: () => void,
    ) => {
      const folded = await viewFromHistoryChunked(
        r.history,
        r.usage,
        // Global baseIndex — items[0] sits at historyStart in the full
        // log, so hi = historyStart + i. baseIndex=0 collided with real
        // page indices → duplicate t{hi} turn/mark ids (multiple active
        // rail marks + key remount churn).
        (r.history_total ?? r.history.length) - r.history.length,
      );
      // Cold view rebuild — the parked queue is session state persisted in
      // SessionMeta; seed it so the composer shows the same rows the actor
      // still holds (live updates continue via QueuedPrompts events).
      folded.queuedPrompts = r.queued_prompts ?? [];
      loadStamp("fold");
      alsoCommit?.();
      loadView(root, id, folded, {
        historyStart: (r.history_total ?? r.history.length) - r.history.length,
        historyTotal: r.history_total,
        turnTotal: r.turn_total,
        turnOffset: r.turn_offset,
      });
      releaseLoading();
    },
    [loadView, releaseLoading],
  );

  // Non-null while the delete confirmation dialog is open — holds the row
  // snapshot taken at click time so the dialog still shows the right title
  // even if the session list refreshes in between.
  const [pendingDelete, setPendingDelete] = useState<SessionRow | null>(null);
  // Same pattern for the fork confirmation — duplicating a session's
  // history is additive but irreversible, so it confirms first.
  const [pendingFork, setPendingFork] = useState<SessionRow | null>(null);
  const [pendingRemoveWs, setPendingRemoveWs] = useState<ProjectOverview | null>(null);
  // Rename dialog — the row being renamed (with its workspace) + the draft.
  const [pendingRename, setPendingRename] = useState<{ root: string; row: SessionRow } | null>(null);
  // Archive confirmation — the row being archived and its workspace.
  const [pendingArchive, setPendingArchive] = useState<{ root: string; row: SessionRow } | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  // Settings is an overlay layer, not a page swap — the workspace stays
  // mounted underneath, so closing it costs nothing (the old takeover
  // unmounted ChatPage and remounted the whole stream on return).
  const [settingsOpen, setSettingsOpen] = useState(false);
  // Changes panel docked right of the chat column; its content reads the
  // active view's `changes`, so it follows the foreground session.
  const [changesOpen, setChangesOpen] = useState(false);
  // Raw-JSON history viewer — fetches the full persisted record on open.
  const [rawOpen, setRawOpen] = useState(false);
  const [rawData, setRawData] = useState<import("@/types").ChatMessage[] | null>(null);
  const [rawLoading, setRawLoading] = useState(false);
  const handleShowRaw = useCallback(async () => {
    if (!activeId) return;
    setRawOpen(true);
    setRawLoading(true);
    try {
      setRawData(await historyRaw(activeId));
    } catch {
      setRawData(null);
    } finally {
      setRawLoading(false);
    }
  }, [activeId]);

  useEffect(() => {
    void getWorkspaceInfo().then((ws) => {
      if (ws) setWorkspace(ws);
    });
  }, []);
  // Hold the skeleton until boot resume resolves — no empty-pane flash.
  const [wsReady, setWsReady] = useState(false);
  useEffect(() => {
    // Keep the recents list even when no workspace is open — the welcome page
    // offers them as one-click reopen, and this is the only place that reads
    // them before a project exists. `root`/`name` stay untouched: an
    // already-active workspace is adopted through the normal open/switch
    // flows, which also sync `activeId`.
    void getWorkspaceInfo()
      .then((ws) => {
        setWorkspace((prev) => ({ ...prev, recents: ws.recents }));
        setWsReady(true);
      })
      .catch(() => setWsReady(true));
  }, []);

  const handlePickWorkspace = async () => {
    setViewLoading(true);
    // A rejected IPC must still lower the veil — otherwise it blocks the
    // whole window with no way out.
    const res = await pickWorkspace().catch(() => null);
    if (res) {
      setWorkspace((prev) => ({ ...prev, root: res.root, name: res.name }));
      // Views are keyed `root:id` — the outgoing workspace's buffers stay
      // cached (and keep streaming while a turn is live there), so
      // switching back restores the live stream, not a store snapshot.
      setActiveId(res.active);
      if (res.history.length > 0) {
        mountLoadedView(res.root, res.active, res);
      }
      void refresh({ force: true });
      void getWorkspaceInfo().then((ws) => {
        if (ws) setWorkspace(ws);
      });
    }
    if (!res || res.history.length === 0) releaseLoading();
  };

  // `veil` is set only by the 编程 ↔ 工作 mode tab. A plain workspace switch
  // (project row, recents, folder picker) keeps to the sidebar and chat pane.
  const handleSwitchWorkspace = async (path: string, { veil = false } = {}) => {
    if (!path || path === workspace.root) return;
    // Flip the mode NOW — the tab (and `officeMode`, which derives from the
    // root) follows the click at once; only the content waits on the kernel
    // load. Same pattern as `handleOpenSession`. Rolled back on failure so
    // the tab never sits on a workspace that didn't open.
    const previousRoot = workspace.root;
    setWorkspace((prev) => ({ ...prev, root: path }));
    setViewLoading(true);
    if (veil) setSwitchVeil(true);
    // Same guard: a rejected switch falls through to the failure branch,
    // which restores the previous root and lowers the veil.
    const res = await switchWorkspace(path).catch(() => null);
    if (res) {
      // The project tree is fetched now (it needs the switched `current`
      // flag) but committed with the chat view below, so the sidebar and the
      // conversation appear in one frame instead of the content leading.
      const tree = await fetchProjects();
      const commitTree = () => {
        if (tree) commit(tree);
      };
      setWorkspace((prev) => ({ ...prev, root: res.root, name: res.name }));
      setActiveId(res.active);
      if (res.history.length > 0) {
        await mountLoadedView(res.root, res.active, res, commitTree);
      } else {
        commitTree();
      }
      void getWorkspaceInfo().then((ws) => {
        if (ws) setWorkspace(ws);
      });
    } else {
      setWorkspace((prev) => ({ ...prev, root: previousRoot }));
      if (veil) setSwitchVeil(false);
    }
    if (!res || res.history.length === 0) releaseLoading();
  };

  // The 编程 ↔ 工作 tab — the only entry that shows the full-window veil.
  const handleSwitchMode = (path: string) => handleSwitchWorkspace(path, { veil: true });

  // Open a conversation from anywhere in the sidebar tree. Ids are
  // per-workspace, so a row belonging to another project switches the
  // active workspace first — its actors get parked (kept alive), not
  // killed, so a running turn survives the switch.
  const handleOpenSession = async (root: string, id: number) => {
    loadReset();
    setViewLoading(true);
    // Flip the pointer NOW — the clicked session's empty view +
    // skeleton replace the old content in the same frame instead of
    // the old conversation lingering through the whole kernel load.
    setActiveId(id);
    if (root && root !== workspace.root) {
      const res = await switchWorkspace(root).catch(() => null);
      if (!res) {
        setViewLoading(false);
        return;
      }
      setWorkspace((prev) => ({ ...prev, root: res.root, name: res.name }));
      void getWorkspaceInfo().then((ws) => {
        if (ws) setWorkspace(ws);
      });
    }
    const r = await openSession(id).catch(() => null);
    loadStamp("ipc:openSession");
    if (!r) {
      // Failed open — nothing to mount; drop the veil and the skeleton.
      setViewLoading(false);
      setSwitchVeil(false);
    } else if (r.history.length > 0) {
      // `root` (the clicked row's project) is the canonical workspace
      // after any switch above — key the rebuilt view under it.
      mountLoadedView(root || workspace.root, id, r);
    } else {
      releaseLoading();
    }
  };

  const handleNewSession = async () => {
    const newId = await newSession();
    if (typeof newId === "number") {
      setActiveId(newId);
    }
  };

  const handleDeleteSession = (id: number) => {
    setViewLoading(true);
    // Evict the cached view NOW — `loadView` never overwrites an existing
    // entry, so a stale buffer would resurface the deleted session the
    // next time its key is mounted.
    dropView(workspace.root, id);
    void deleteSession(id).then((r) => {
      void refresh({ force: true });
      // If the deleted session was on screen, the backend already
      // switched to another — follow it and rebuild its view.
      setActiveId(r.active);
      if (r.history.length > 0) mountLoadedView(workspace.root, r.active, r);
      else releaseLoading();
    }).catch(() => setViewLoading(false));
  };

  /** Older-history page — the stream's scroll-top trigger calls this;
   * folds the slice with a `baseIndex` so every item carries its global
   * `hi` and turn keys stay stable across prepends. */
  const handleLoadOlder = useCallback(async () => {
    if (!workspace.root || !activeId) return false;
    // Read through the ref — this callback is re-invoked by the seek
    // loop and by scroll events while older closure copies are still
    // alive; the render-captured `active` would hand every call the
    // same stale `before`.
    const start = activeViewNow().historyStart ?? 0;
    if (start <= 0) return false;
    const r = await historyPage(activeId, start);
    if (r.history.length === 0) return false;
    const folded = await viewFromHistoryChunked(r.history, undefined, start - r.history.length);
    // The prepend commit mounts the whole new page in one commit (the
    // scroll-restore needs real heights) — a transition keeps its render
    // interruptible so the stream stays clickable while it lands.
    startTransition(() =>
      prependItems(workspace.root, activeId, folded.items, start - r.history.length, r.turn_offset),
    );
    return true;
  }, [workspace.root, activeId, prependItems]);

  // First-load: the kernel resumed the most recent session at boot, but
  // the webview only sees *new* events — rebuild the stream from the
  // persisted snapshot so the resumed session isn't blank. `openSession`
  // is idempotent for a live actor; it only re-reads the store here.
  const bootLoaded = useRef(false);
  useEffect(() => {
    if (bootLoaded.current || sessions.length === 0) return;
    bootLoaded.current = true;
    const current = sessions.find((s) => s.active) ?? sessions[0];
    // The canonical root from the project tree — `workspace.root` state
    // may still be empty this early in the boot sequence.
    const currentRoot = projects.find((p) => p.current)?.root ?? "";
    setViewLoading(true);
    void openSession(current.id).then((r) => {
      if (r && r.history.length > 0)
        mountLoadedView(currentRoot, current.id, r);
      else releaseLoading();
    });
    setActiveId(current.id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessions]);

  const activeSession =
    sessions.find((s) => (activeId ? s.id === activeId : s.active)) || sessions[0];
  const sessionTitle =
    sessions.length === 0
      ? "" // zero sessions: no phantom "新会话" in the title bar
      : activeSession?.title === "new session" || !activeSession?.title
        ? "新会话"
        : activeSession.title;
  const pendingDeleteTitle = !pendingDelete
    ? ""
    : pendingDelete.title === "new session" || !pendingDelete.title
      ? "新会话"
      : pendingDelete.title;
  const pendingArchiveTitle = !pendingArchive
    ? ""
    : pendingArchive.row.title === "new session" || !pendingArchive.row.title
      ? "新会话"
      : pendingArchive.row.title;
  const pendingForkTitle = !pendingFork
    ? ""
    : pendingFork.title === "new session" || !pendingFork.title
      ? "新会话"
      : pendingFork.title;

  // Removing a project always confirms — the copy says what happens to its
  // records, and warns louder when a running task would be interrupted.
  const requestRemoveWorkspace = (p: ProjectOverview) => setPendingRemoveWs(p);

  const doRemoveWorkspace = async (root: string) => {
    const res = await removeWorkspace(root).catch(() => null);
    void refresh({ force: true });
    if (res?.removed_active) {
      setActiveId(0);
    }
    const ws = await getWorkspaceInfo().catch(() => null);
    if (ws) setWorkspace(ws);
  };

  // Meta edits: run, then re-read the tree; failures surface as a toast.
  const editSession = (op: Promise<unknown>) => {
    void op
      .then(() => void refresh({ force: true }))
      .catch((e) => toast.error(String(e)));
  };

  const sessionActions: SessionActions = {
    rename: (root, row) => {
      setRenameDraft(row.title === "new session" ? "" : row.title);
      setPendingRename({ root, row });
    },
    pin: (root, id, pinned) => editSession(pinSession(root, id, pinned)),
    pinGlobal: (root, id, pinned) => editSession(pinSessionGlobal(root, id, pinned)),
    // Archiving hides the session and drops its pins — confirm first.
    // Restoring is harmless and runs directly.
    archive: (root, row, archived) => {
      if (archived) setPendingArchive({ root, row });
      else editSession(archiveSession(root, row.id, false));
    },
    exportRecord: (root, id) => {
      void exportSession(root, id)
        .then((r) => {
          if (r.saved) toast.success(`已导出到 ${r.path}`);
        })
        .catch((e) => toast.error(String(e)));
    },
    fork: (id) => {
      // Fork duplicates history — confirm before touching the store,
      // same pattern as the delete dialog below.
      const row = sessions.find((s) => s.id === id);
      if (row) setPendingFork(row);
    },
    delete: (id) => {
      // Deletion is destructive — park the row and let the dialog
      // below confirm before touching the backend.
      const row = sessions.find((s) => s.id === id);
      if (row) setPendingDelete(row);
    },
  };

  const submitRename = () => {
    if (!pendingRename) return;
    editSession(renameSession(pendingRename.root, pendingRename.row.id, renameDraft));
    setPendingRename(null);
  };

  const doFork = (id: number) => {
    // Backend activates the fork — mirror it locally + rebuild the
    // stream view from the copied history.
    setViewLoading(true);
    void forkSession(id).then((r) => {
      if (!r) {
        setViewLoading(false);
        return;
      }
      void refresh({ force: true });
      setActiveId(r.id);
      if (r.history.length > 0) mountLoadedView(workspace.root, r.id, r);
      else releaseLoading();
    }).catch(() => setViewLoading(false));
  };

  // The backend reports `running` from live handles (including parked
  // workspaces); overlaying `runningKeys` (fresh off `StateChanged`
  // events) makes the orb appear the moment a turn starts instead of up
  // to 2s later. Keys are `root:id` — no collision across projects, so a
  // background workspace's running turn keeps its orb too.
  const sidebarProjects = projects.map((p) => ({
    ...p,
    sessions: p.sessions.map((s) => ({
      ...s,
      running: s.running || runningKeys.has(`${p.root}:${s.id}`),
    })),
  }));

  // Office mode IS a workspace (the `~/.local/share/husk/office`
  // pseudo-workspace) — the composer only offers「办公」in its mode
  // dropdown while that workspace is active; everywhere else switching
  // happens through the sidebar's 办公 block.
  const officeMode =
    !!workspace.office_root && workspace.root === workspace.office_root;

  return (
    <>
      <MainLayout
        sidebar={
        <SessionSidebar
          projects={sidebarProjects}
          projectsLoaded={sessionsLoaded}
          activeId={activeId}
          hasWorkspace={wsReady && !!workspace.root}
          onNew={handleNewSession}
          onOpenSession={handleOpenSession}
          actions={sessionActions}
          onPickWorkspace={handlePickWorkspace}
          onSwitchWorkspace={(path) => void handleSwitchWorkspace(path)}
          onSwitchMode={(path) => void handleSwitchMode(path)}
          onRemoveWorkspace={requestRemoveWorkspace}
          onOpenSettings={() => setSettingsOpen(true)}
        />
        }
      >
        <ChatPage
          title={sessionTitle}
          workspaceRoot={workspace.root}
          officeMode={officeMode}
          loading={viewLoading}
          sessionKey={`${workspace.root}:${activeId}`}
          sessionsEmpty={sessionsLoaded && sessions.length === 0}
          onNewSession={handleNewSession}
          onLoadOlder={handleLoadOlder}
          onShowRaw={handleShowRaw}
          onOpenWorkspace={handlePickWorkspace}
          recents={workspace.recents}
          onOpenRecent={(path) => void handleSwitchWorkspace(path)}
          workspaceReady={wsReady}
          changesOpen={changesOpen}
          onToggleChanges={() => setChangesOpen((o) => !o)}
        />
      </MainLayout>

      {/* Switch veil — same look as the boot splash, over sidebar and chat
          together so the two appear as one. Driven by the view-loading flag. */}
      <WorkspaceLoadingVeil visible={switchVeil} />

      {/* Settings overlay — floats above the workspace, which keeps its
          DOM (and the expensive stream tree) mounted the whole time.
          z-40 on purpose: Radix dropdowns/popovers portal at z-50 and
          must render ABOVE the settings pane — a higher overlay hides
          every KV popup inside it. */}
      {settingsOpen && (
        <div className="fixed inset-0 z-40">
          <SettingsPage onClose={() => setSettingsOpen(false)} />
        </div>
      )}

      {/* Raw-JSON history viewer — opened from the title-bar icon. */}
      <RawHistoryDialog
        open={rawOpen}
        onOpenChange={setRawOpen}
        messages={rawData}
        loading={rawLoading}
      />

      {/* Fork confirmation — creates a copy of the session's history. */}
      <Dialog
        open={pendingFork !== null}
        onOpenChange={(open) => {
          if (!open) setPendingFork(null);
        }}
      >
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle className="text-base">Fork 会话</DialogTitle>
            <DialogDescription>
              确定要复制会话「{pendingForkTitle}」的完整对话记录，创建一个新会话吗？
            </DialogDescription>
          </DialogHeader>
          <DialogFooter className="gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => setPendingFork(null)}
            >
              取消
            </Button>
            <Button
              size="sm"
              onClick={() => {
                if (!pendingFork) return;
                doFork(pendingFork.id);
                setPendingFork(null);
              }}
            >
              Fork
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={pendingRemoveWs !== null}
        onOpenChange={(open) => {
          if (!open) setPendingRemoveWs(null);
        }}
      >
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle className="text-base">移除项目</DialogTitle>
            <DialogDescription>
              {pendingRemoveWs?.sessions.some((r) => r.running)
                ? `项目「${pendingRemoveWs?.name}」有任务正在运行，移除会立即中断这些任务。`
                : `确定从列表中移除项目「${pendingRemoveWs?.name}」吗？`}
              会话记录会保留，重新打开该文件夹即可恢复。
            </DialogDescription>
          </DialogHeader>
          <DialogFooter className="gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => setPendingRemoveWs(null)}
            >
              取消
            </Button>
            <Button
              variant="destructive"
              size="sm"
              onClick={() => {
                if (!pendingRemoveWs) return;
                void doRemoveWorkspace(pendingRemoveWs.root);
                setPendingRemoveWs(null);
              }}
            >
              移除
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={pendingRename !== null}
        onOpenChange={(open) => {
          if (!open) setPendingRename(null);
        }}
      >
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle className="text-base">重命名会话</DialogTitle>
            <DialogDescription>留空则恢复为自动生成的标题。</DialogDescription>
          </DialogHeader>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              submitRename();
            }}
          >
            <Input
              autoFocus
              value={renameDraft}
              maxLength={80}
              onChange={(e) => setRenameDraft(e.target.value)}
              placeholder="会话标题"
              className="h-8 text-sm"
            />
            <DialogFooter className="gap-2 mt-4">
              <Button type="button" variant="outline" size="sm" onClick={() => setPendingRename(null)}>
                取消
              </Button>
              <Button type="submit" size="sm">
                保存
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <Dialog
        open={pendingArchive !== null}
        onOpenChange={(open) => {
          if (!open) setPendingArchive(null);
        }}
      >
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle className="text-base">归档会话</DialogTitle>
            <DialogDescription>
              确定要归档会话「{pendingArchiveTitle}」吗？归档后会从列表中隐藏并取消置顶，可在列表底部的「已归档」中恢复。
            </DialogDescription>
          </DialogHeader>
          <DialogFooter className="gap-2">
            <Button variant="outline" size="sm" onClick={() => setPendingArchive(null)}>
              取消
            </Button>
            <Button
              size="sm"
              onClick={() => {
                if (!pendingArchive) return;
                editSession(archiveSession(pendingArchive.root, pendingArchive.row.id, true));
                setPendingArchive(null);
              }}
            >
              归档
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={pendingDelete !== null}
        onOpenChange={(open) => {
          if (!open) setPendingDelete(null);
        }}
      >
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle className="text-base">删除会话</DialogTitle>
            <DialogDescription>
              确定要删除会话「{pendingDeleteTitle}」吗？此操作无法撤销，会话记录将被永久删除。
            </DialogDescription>
          </DialogHeader>
          <DialogFooter className="gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => setPendingDelete(null)}
            >
              取消
            </Button>
            <Button
              variant="destructive"
              size="sm"
              onClick={() => {
                if (!pendingDelete) return;
                handleDeleteSession(pendingDelete.id);
                setPendingDelete(null);
              }}
            >
              删除
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Keep last in the tree: sonner renders its <ol> in place here, so
          if its stylesheet ever fails to apply the unstyled element lands
          below the shell instead of shoving it down from the top. */}
      <Toaster />
    </>
  );
}
