import { useCallback, useEffect, useRef, useState } from "react";
import { Toaster } from "@/components/ui/sonner";
import { useAgentEvents } from "./hooks/use-agent-events";
import { useAgentSession } from "./hooks/use-agent-session";
import { viewFromHistory, viewFromHistoryChunked } from "./hooks/view-from-history";
import { loadReset, loadStamp } from "./lib/load-probe";
import { getWorkspaceInfo, pickWorkspace, switchWorkspace, removeWorkspace, forkSession, deleteSession, pinSession, historyPage, historyRaw, type WorkspaceInfo } from "./invoke/agent";
import type { ProjectOverview } from "./types";
import type { SessionRow } from "./types";
import { SessionSidebar } from "./components/session-sidebar";
import { MainLayout } from "./layout/main-layout";
import { ChatPage } from "./pages/chat";
import { SettingsPage } from "./pages/settings";
import { RawHistoryDialog } from "./components/raw-history-dialog";
import { Button } from "@/components/ui/button";
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
 *  - devtools shortcuts are dead keys in production builds.
 * Dev (`vite dev`) keeps both for debugging. */
function useAppChromeGuards() {
  useEffect(() => {
    const onContextMenu = (e: MouseEvent) => {
      // Dev keeps the native webview menu — right-click inspect is the
      // debugger's front door on WebKitGTK.
      if (import.meta.env.DEV) return;
      // Radix ContextMenuTrigger calls preventDefault itself; letting the
      // event through to a trigger is what opens our custom menu.
      if (isEditableTarget(e)) return;
      const el = e.target as HTMLElement | null;
      if (el?.closest?.("[data-allow-contextmenu]")) return;
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
      const devtools =
        k === "F12" ||
        (mod && e.shiftKey && ["i", "I", "j", "J", "c", "C", "k", "K"].includes(k)) ||
        (mod && e.altKey && ["i", "I", "j", "J", "c", "C"].includes(k)) ||
        (e.ctrlKey && (k === "u" || k === "U"));
      if (devtools) e.preventDefault();
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
  const {
    active,
    activeId,
    setActiveId,
    loadView,
    prependItems,
    runningKeys,
    gitInfo,
    ctxWindow,
    modelCost,
  } = useAgentEvents(workspace.root);
  const { projects, sessions, refresh, newSession, openSession } = useAgentSession();
  // True while a session/workspace switch is fetching history + rebuilding
  // the view — the stream renders a skeleton instead of a stale/empty pane.
  const [viewLoading, setViewLoading] = useState(false);
  // Drop the loading veil only AFTER the mounted tree has committed and
  // painted — session loads full-mount every turn in one synchronous
  // commit, and if the veil lifts inside that same commit the app shows
  // a frozen half-frame instead of "loading → ready". Two rAFs carries
  // the flag past the next paint boundary.
  const releaseLoading = useCallback(() => {
    requestAnimationFrame(() =>
      requestAnimationFrame(() => setViewLoading(false)),
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
        usage?: Parameters<typeof viewFromHistory>[1];
      },
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
      loadStamp("fold");
      loadView(root, id, folded, {
        historyStart: (r.history_total ?? r.history.length) - r.history.length,
        historyTotal: r.history_total,
        turnTotal: r.turn_total,
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
  // Settings is an overlay layer, not a page swap — the workspace stays
  // mounted underneath, so closing it costs nothing (the old takeover
  // unmounted ChatPage and remounted the whole stream on return).
  const [settingsOpen, setSettingsOpen] = useState(false);
  // Raw-JSON history viewer — fetches the full persisted record on open.
  const [rawOpen, setRawOpen] = useState(false);
  const [rawData, setRawData] = useState<import("./types").ChatMessage[] | null>(null);
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
    void getWorkspaceInfo().then(() => setWsReady(true)).catch(() => setWsReady(true));
  }, []);

  const handlePickWorkspace = async () => {
    setViewLoading(true);
    const res = await pickWorkspace();
    if (res) {
      setWorkspace((prev) => ({ ...prev, root: res.root, name: res.name }));
      // Views are keyed `root:id` — the outgoing workspace's buffers stay
      // cached (and keep streaming while a turn is live there), so
      // switching back restores the live stream, not a store snapshot.
      setActiveId(res.active);
      if (res.history.length > 0) {
        mountLoadedView(res.root, res.active, res);
      }
      void refresh();
      void getWorkspaceInfo().then((ws) => {
        if (ws) setWorkspace(ws);
      });
    }
    if (!res || res.history.length === 0) releaseLoading();
  };

  const handleSwitchWorkspace = async (path: string) => {
    if (!path || path === workspace.root) return;
    setViewLoading(true);
    const res = await switchWorkspace(path);
    if (res) {
      setWorkspace((prev) => ({ ...prev, root: res.root, name: res.name }));
      setActiveId(res.active);
      if (res.history.length > 0) {
        mountLoadedView(res.root, res.active, res);
      }
      void refresh();
      void getWorkspaceInfo().then((ws) => {
        if (ws) setWorkspace(ws);
      });
    }
    if (!res || res.history.length === 0) releaseLoading();
  };

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
      const res = await switchWorkspace(root);
      if (!res) {
        setViewLoading(false);
        return;
      }
      setWorkspace((prev) => ({ ...prev, root: res.root, name: res.name }));
      void getWorkspaceInfo().then((ws) => {
        if (ws) setWorkspace(ws);
      });
    }
    const r = await openSession(id);
    loadStamp("ipc:openSession");
    if (r && r.history.length > 0) {
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
    void deleteSession(id).then((r) => {
      void refresh();
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
    const start = active?.historyStart ?? 0;
    if (start <= 0) return false;
    const r = await historyPage(activeId, start);
    if (r.history.length === 0) return false;
    const folded = await viewFromHistoryChunked(r.history, undefined, start - r.history.length);
    prependItems(workspace.root, activeId, folded.items, start - r.history.length);
    return true;
  }, [workspace.root, activeId, active?.historyStart, prependItems]);

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
    activeSession?.title === "new session" || !activeSession?.title
      ? "新会话"
      : activeSession.title;
  const pendingDeleteTitle = !pendingDelete
    ? ""
    : pendingDelete.title === "new session" || !pendingDelete.title
      ? "新会话"
      : pendingDelete.title;
  const pendingForkTitle = !pendingFork
    ? ""
    : pendingFork.title === "new session" || !pendingFork.title
      ? "新会话"
      : pendingFork.title;

  const requestRemoveWorkspace = (p: ProjectOverview) => {
    // A running turn dies with it — confirm first.
    if (p.sessions.some((r) => r.running)) {
      setPendingRemoveWs(p);
      return;
    }
    void doRemoveWorkspace(p.root);
  };

  const doRemoveWorkspace = async (root: string) => {
    const res = await removeWorkspace(root).catch(() => null);
    void refresh();
    if (res?.removed_active) {
      setActiveId(0);
    }
    const ws = await getWorkspaceInfo().catch(() => null);
    if (ws) setWorkspace(ws);
  };

  const handlePinSession = (id: number, pinned: boolean) => {
    void pinSession(id, pinned).then(() => void refresh()).catch(() => {});
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
      void refresh();
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

  return (
    <>
      <Toaster />
      <MainLayout
        sidebar={
        <SessionSidebar
          projects={sidebarProjects}
          activeId={activeId}
          hasWorkspace={wsReady && !!workspace.root}
          onNew={handleNewSession}
          onOpenSession={handleOpenSession}
          onFork={(id) => {
            // Fork duplicates history — confirm before touching the store,
            // same pattern as the delete dialog below.
            const row = sessions.find((s) => s.id === id);
            if (row) setPendingFork(row);
          }}
          onPin={handlePinSession}
          onDelete={(id) => {
            // Deletion is destructive — park the row and let the dialog
            // below confirm before touching the backend.
            const row = sessions.find((s) => s.id === id);
            if (row) setPendingDelete(row);
          }}
          onPickWorkspace={handlePickWorkspace}
          onSwitchWorkspace={handleSwitchWorkspace}
          onRemoveWorkspace={requestRemoveWorkspace}
          onOpenSettings={() => setSettingsOpen(true)}
        />
        }
      >
        <ChatPage
          title={sessionTitle}
          view={active}
          workspaceRoot={workspace.root}
          gitInfo={gitInfo}
          contextWindowHint={ctxWindow}
          modelCost={modelCost}
          loading={viewLoading}
          sessionKey={`${workspace.root}:${activeId}`}
          hasMore={(active?.historyStart ?? 0) > 0}
          onLoadOlder={handleLoadOlder}
          onShowRaw={handleShowRaw}
          onOpenWorkspace={handlePickWorkspace}
          workspaceReady={wsReady}
        />
      </MainLayout>

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
        title={sessionTitle}
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
              项目「{pendingRemoveWs?.name}」有任务正在运行，移除会立即中断这些任务。会话记录会保留，重新打开该文件夹即可恢复。确定移除吗？
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
    </>
  );
}
