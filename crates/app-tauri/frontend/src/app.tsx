import { useEffect, useRef, useState } from "react";
import { useAgentEvents, useAgentSession, viewFromHistory } from "./hooks/useAgent";
import { getWorkspaceInfo, pickWorkspace, switchWorkspace, openSettingsWindow, forkSession, deleteSession, type WorkspaceInfo } from "./invoke/agent";
import { TitleBar } from "./components/title-bar";
import { SessionSidebar } from "./components/session-sidebar";
import { ChatStream } from "./components/chat-stream";
import { ComposerBar } from "./components/composer-bar";

export function App() {
  const { active, activeId, setActiveId, loadView, runningIds } = useAgentEvents();
  const { sessions, refresh, newSession, openSession } = useAgentSession();
  const [workspace, setWorkspace] = useState<WorkspaceInfo>({ root: "", name: "", recents: [] });

  useEffect(() => {
    void getWorkspaceInfo().then((ws) => {
      if (ws) setWorkspace(ws);
    });
  }, []);

  const handlePickWorkspace = async () => {
    const res = await pickWorkspace();
    if (res) {
      setWorkspace((prev) => ({ ...prev, root: res.root, name: res.name }));
      setActiveId(res.active);
      if (res.history.length > 0) {
        loadView(res.active, viewFromHistory(res.history));
      }
      void getWorkspaceInfo().then((ws) => {
        if (ws) setWorkspace(ws);
      });
    }
  };

  const handleSwitchWorkspace = async (path: string) => {
    const res = await switchWorkspace(path);
    if (res) {
      setWorkspace((prev) => ({ ...prev, root: res.root, name: res.name }));
      setActiveId(res.active);
      if (res.history.length > 0) {
        loadView(res.active, viewFromHistory(res.history));
      }
      void getWorkspaceInfo().then((ws) => {
        if (ws) setWorkspace(ws);
      });
    }
  };

  const handleNewSession = async () => {
    const newId = await newSession();
    if (typeof newId === "number") {
      setActiveId(newId);
    }
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
      if (r && r.history.length > 0) loadView(current.id, viewFromHistory(r.history));
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

  return (
    <div className="flex h-full w-full bg-white overflow-hidden select-none">
      <SessionSidebar
        // `runningIds` (live, from StateChanged events) overlays the polled
        // `row.running` — the orb appears the moment a turn starts rather
        // than up to 2s later, and survives a refresh racing the flag.
        sessions={sessions.map((s) => ({
          ...s,
          running: s.running || runningIds.has(s.id),
        }))}
        activeId={activeId}
        workspaceName={workspace.name}
        workspaceRoot={workspace.root}
        recentWorkspaces={workspace.recents}
        onNew={handleNewSession}
        onOpen={(id) => {
          void openSession(id).then((r) => {
            if (r && r.history.length > 0) loadView(id, viewFromHistory(r.history));
          });
          setActiveId(id);
        }}
        onFork={(id) => {
          // Backend activates the fork — mirror it locally + rebuild the
          // stream view from the copied history.
          void forkSession(id).then((r) => {
            if (!r) return;
            void refresh();
            setActiveId(r.id);
            if (r.history.length > 0) loadView(r.id, viewFromHistory(r.history));
          }).catch(() => {});
        }}
        onDelete={(id) => {
          void deleteSession(id).then((r) => {
            void refresh();
            // If the deleted session was on screen, the backend already
            // switched to another — follow it and rebuild its view.
            setActiveId(r.active);
            if (r.history.length > 0) loadView(r.active, viewFromHistory(r.history));
          }).catch(() => {});
        }}
        onPickWorkspace={handlePickWorkspace}
        onSwitchWorkspace={handleSwitchWorkspace}
        onOpenSettings={() => void openSettingsWindow()}
      />
      <div className="main-col h-full overflow-hidden">
        <TitleBar title={sessionTitle} />
        <ChatStream view={active} />
        <ComposerBar view={active} workspaceRoot={workspace.root} />
      </div>
    </div>
  );
}
