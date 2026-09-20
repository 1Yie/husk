import { useEffect, useRef, useState } from "react";
import { useAgentEvents } from "./hooks/use-agent-events";
import { useAgentSession } from "./hooks/use-agent-session";
import { viewFromHistory } from "./hooks/view-from-history";
import { getWorkspaceInfo, pickWorkspace, switchWorkspace, openSettingsWindow, forkSession, deleteSession, type WorkspaceInfo } from "./invoke/agent";
import type { SessionRow } from "./types";
import { SessionSidebar } from "./components/session-sidebar";
import { MainLayout } from "./layout/main-layout";
import { ChatPage } from "./pages/chat";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

export function App() {
  const {
    active,
    activeId,
    setActiveId,
    loadView,
    clearViews,
    runningIds,
    gitInfo,
    ctxWindow,
  } = useAgentEvents();
  const { projects, sessions, refresh, newSession, openSession } = useAgentSession();
  const [workspace, setWorkspace] = useState<WorkspaceInfo>({ root: "", name: "", recents: [] });
  // Non-null while the delete confirmation dialog is open — holds the row
  // snapshot taken at click time so the dialog still shows the right title
  // even if the session list refreshes in between.
  const [pendingDelete, setPendingDelete] = useState<SessionRow | null>(null);

  useEffect(() => {
    void getWorkspaceInfo().then((ws) => {
      if (ws) setWorkspace(ws);
    });
  }, []);

  const handlePickWorkspace = async () => {
    const res = await pickWorkspace();
    if (res) {
      setWorkspace((prev) => ({ ...prev, root: res.root, name: res.name }));
      // Session ids are per-workspace — drop cached streams before the new
      // workspace's history lands, or a colliding id keeps showing the old
      // project's conversation.
      clearViews();
      setActiveId(res.active);
      if (res.history.length > 0) {
        loadView(res.active, viewFromHistory(res.history, res.usage));
      }
      void refresh();
      void getWorkspaceInfo().then((ws) => {
        if (ws) setWorkspace(ws);
      });
    }
  };

  const handleSwitchWorkspace = async (path: string) => {
    if (!path || path === workspace.root) return;
    const res = await switchWorkspace(path);
    if (res) {
      setWorkspace((prev) => ({ ...prev, root: res.root, name: res.name }));
      clearViews();
      setActiveId(res.active);
      if (res.history.length > 0) {
        loadView(res.active, viewFromHistory(res.history, res.usage));
      }
      void refresh();
      void getWorkspaceInfo().then((ws) => {
        if (ws) setWorkspace(ws);
      });
    }
  };

  // Open a conversation from anywhere in the sidebar tree. Ids are
  // per-workspace, so a row belonging to another project switches the
  // active workspace first (the kernel only ever has actors for one).
  const handleOpenSession = async (root: string, id: number) => {
    if (root && root !== workspace.root) {
      const res = await switchWorkspace(root);
      if (!res) return;
      setWorkspace((prev) => ({ ...prev, root: res.root, name: res.name }));
      clearViews();
      void getWorkspaceInfo().then((ws) => {
        if (ws) setWorkspace(ws);
      });
    }
    const r = await openSession(id);
    if (r) {
      setActiveId(id);
      if (r.history.length > 0) loadView(id, viewFromHistory(r.history, r.usage));
    }
  };

  const handleNewSession = async () => {
    const newId = await newSession();
    if (typeof newId === "number") {
      setActiveId(newId);
    }
  };

  const handleDeleteSession = (id: number) => {
    void deleteSession(id).then((r) => {
      void refresh();
      // If the deleted session was on screen, the backend already
      // switched to another — follow it and rebuild its view.
      setActiveId(r.active);
      if (r.history.length > 0) loadView(r.active, viewFromHistory(r.history, r.usage));
    }).catch(() => {});
  };

  // First-load: the kernel resumed the most recent session at boot, but
  // the webview only sees *new* events — rebuild the stream from the
  // persisted snapshot so the resumed session isn't blank. `openSession`
  // is idempotent for a live actor; it only re-reads the store here.
  const bootLoaded = useRef(false);
  useEffect(() => {
    if (bootLoaded.current || sessions.length === 0) return;
    bootLoaded.current = true;
    const current = sessions.find((s) => s.active) ?? sessions[0];
    void openSession(current.id).then((r) => {
      if (r && r.history.length > 0) loadView(current.id, viewFromHistory(r.history, r.usage));
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

  // The backend reports `running` from live handles; overlaying `runningIds`
  // (fresh off `StateChanged` events) makes the orb appear the moment a turn
  // starts instead of up to 2s later. Only the current project has live ids —
  // another project's row may collide on id, so it keeps the polled value.
  const sidebarProjects = projects.map((p) =>
    p.current
      ? {
          ...p,
          sessions: p.sessions.map((s) => ({
            ...s,
            running: s.running || runningIds.has(s.id),
          })),
        }
      : p
  );

  return (
    <>
      <MainLayout
        sidebar={
        <SessionSidebar
          projects={sidebarProjects}
          activeId={activeId}
          onNew={handleNewSession}
          onOpenSession={handleOpenSession}
          onFork={(id) => {
            // Backend activates the fork — mirror it locally + rebuild the
            // stream view from the copied history.
            void forkSession(id).then((r) => {
              if (!r) return;
              void refresh();
              setActiveId(r.id);
              if (r.history.length > 0) loadView(r.id, viewFromHistory(r.history, r.usage));
            }).catch(() => {});
          }}
          onDelete={(id) => {
            // Deletion is destructive — park the row and let the dialog
            // below confirm before touching the backend.
            const row = sessions.find((s) => s.id === id);
            if (row) setPendingDelete(row);
          }}
          onPickWorkspace={handlePickWorkspace}
          onSwitchWorkspace={handleSwitchWorkspace}
          onOpenSettings={() => void openSettingsWindow()}
        />
        }
      >
        <ChatPage
          title={sessionTitle}
          view={active}
          workspaceRoot={workspace.root}
          gitInfo={gitInfo}
          contextWindowHint={ctxWindow}
        />
      </MainLayout>

      <Dialog
        open={pendingDelete !== null}
        onOpenChange={(open) => {
          if (!open) setPendingDelete(null);
        }}
      >
        <DialogContent className="max-w-sm gap-3 p-4">
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
