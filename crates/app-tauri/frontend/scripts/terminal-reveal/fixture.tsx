import { lazy, Profiler, Suspense, useState } from "react";
import { createRoot } from "react-dom/client";
import "../../src/index.css";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useLayoutStore } from "@/app/layout/layout-store";
import { SessionSidebar } from "@/features/chat/components/session-sidebar";
import { TerminalDock } from "@/features/terminal/terminal-dock";
import { TerminalSession } from "@/features/terminal/terminal-session";
import { DEFAULT_TERMINAL_PREFS, useTerminalPrefs } from "@/features/terminal/preferences";
import { MainLayout } from "@/app/layout/main-layout";
import { ChatPage } from "@/features/chat/pages/chat";
import { useAgentStore } from "@/stores/agent-store";
import { useDockStore } from "@/features/chat/components/dock/state";
import { viewFromHistory } from "@/features/chat/hooks/view-from-history";
import type { ChatMessage } from "@/types";
import { checkTerminalFonts } from "./font-probe";
import { applyAppearance } from "@/lib/appearance";
import { SettingsPage } from "@/features/settings/pages";

const TerminalSettings = lazy(() => import("@/features/settings/pages/terminal")
  .then((module) => ({ default: module.TerminalSettings })));
const AppearanceFixture = lazy(() => import("./appearance-fixture"));
const welcomeResponsive = new URLSearchParams(location.search).has("welcome-responsive");
const workspacePanels = new URLSearchParams(location.search).has("panels");
const welcomeClicks = { open: 0, newSession: 0, recent: 0 };

// Fixture-only access to the real class works in both dev and bundled builds.
(window as unknown as { __terminalTestSession: typeof TerminalSession }).__terminalTestSession = TerminalSession;
(window as unknown as { __checkTerminalFonts: typeof checkTerminalFonts }).__checkTerminalFonts = checkTerminalFonts;

const history: ChatMessage[] = [{ role: "user", content: "长会话中打开终端" }];
for (let i = 0; i < 160; i++) {
  history.push({
    role: "assistant", reasoning: `思考阶段 ${i}：检查代码与边界。`.repeat(20),
    content: i % 5 === 0 ? "分析说明 ".repeat(80) : "",
    tool_calls: [{ id: `k${i}`, type: "function", function: {
      name: "bash", arguments: JSON.stringify({ command: `cargo test -p task-${i}` }),
    } }],
  });
  history.push({ role: "tool", tool_call_id: `k${i}`, content: "ok" });
}
history.push({ role: "assistant", content: "检查完成" });
const workspaceLinks = new URLSearchParams(location.search).has("workspace-links");
const markdownResources = new URLSearchParams(location.search).has("markdown-resources");
const linkHistory: ChatMessage[] = [
  { role: "user", content: "定位文件" },
  { role: "assistant", content: [
    "[file source](file:///repo/src/a.ts#L80C3)",
    "[relative source](src/a.ts:4:2)",
    "[Markdown source](./notes.md#L3C2)",
    "[missing file](missing.ts:1)",
    "[truncated source](long.ts:2050)",
    "[invalid line](src/a.ts:9999)",
    "[large file](large.ts:1)",
    "[slow file](slow.ts:1)",
    "[web](https://example.com)",
    "[unsafe](javascript:alert%281%29)",
    "[outside](file:///outside/secret.ts)",
  ].join("\n\n") },
];
const view = viewFromHistory(workspaceLinks || markdownResources ? linkHistory : history, null, 0);
useTerminalPrefs.setState(DEFAULT_TERMINAL_PREFS);
useLayoutStore.setState({ sidebarOpen: true, terminalOpen: new URLSearchParams(location.search).has("settings-shell") });
useAgentStore.getState().setWorkspaceRoot("/repo");
useAgentStore.getState().setActiveId(1);
useAgentStore.getState().loadView("/repo", 1, view);
useDockStore.setState({ open: !workspaceLinks, tab: markdownResources ? "files" : "tools", browserOpen: markdownResources, width: markdownResources ? 360 : 300 });
Object.assign(window, { workspacePanelsFixture: {
  terminalOpen: (open: boolean) => useLayoutStore.setState({ terminalOpen: open }),
  terminalHeight: (panelHeight: number) => useTerminalPrefs.setState({ panelHeight }),
  snapshot: () => ({ dock: useDockStore.getState(), terminal: useTerminalPrefs.getState() }),
  populateDock: () => {
    useAgentStore.setState(s => ({ views: new Map(s.views).set("/repo:1", { ...view,
      changes: [{ path: "src/long-workspace-panel-name.tsx", kind: "update", patches: [], adds: 1, dels: 0, fuzzy: false }],
      artifacts: [{ path: "out/long-deliverable-name.pdf", name: "long-deliverable-name.pdf", ext: "pdf", tool: "office_create", updatedAt: 0 }],
      bgJobs: [{ id: "fixture-job", command: "fixture command", state: { kind: "done", label: "exited(0)", code: 0 }, started_at_ms: 1, elapsed_ms: 1000, notify: true, has_own_sink: false }],
    }) }));
    useDockStore.getState().setWidth(260);
    useDockStore.setState({ browserOpen: true, tab: "tools" });
  },
  theme: (value: string) => applyAppearance({
    theme_mode: value === "dark" ? "dark" : "light", accent: "#339cff",
    background: value === "custom" ? "#e7dfcc" : "#ffffff", dark_background: "#16161a",
    foreground: "#1a1c1f", ui_font: value === "custom" ? "Georgia" : "system-ui",
    code_font: "monospace", contrast: 45,
  }),
} });
let appRenders = 0;
function App() {
  const [, setParentUpdate] = useState(0);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [officeMode, setOfficeMode] = useState(false);
  const [workspaceRoot, setWorkspaceRoot] = useState("/repo");
  const [sessionId, setSessionId] = useState(1);
  const [welcomeScene, setWelcomeScene] = useState(welcomeResponsive ? "unrooted" : "");
  Object.assign(window, { welcomeResponsiveFixture: {
    scene: (scene: string) => {
      setWelcomeScene(scene);
      setOfficeMode(scene === "office");
      if (scene === "greeting") {
        useAgentStore.setState(state => ({
          views: new Map(state.views).set("/repo:1", viewFromHistory([], null, 0)),
        }));
      }
    },
    panels: (sidebar: boolean, dockWidth: number) => {
      useLayoutStore.setState({ sidebarOpen: sidebar });
      useDockStore.setState({ open: dockWidth > 0, width: dockWidth || 300, tab: "tools" });
    },
    clicks: () => ({ ...welcomeClicks }),
  } });
  Object.assign(window, { workspaceLinksFixture: {
    update: () => setParentUpdate(value => value + 1),
    office: setOfficeMode,
    close: () => useDockStore.getState().setOpen(false),
    switch: (root: string, id: number) => {
      useAgentStore.getState().setWorkspaceRoot(root);
      useAgentStore.getState().setActiveId(id);
      useAgentStore.getState().loadView(root, id, view);
      setWorkspaceRoot(root);
      setSessionId(id);
    },
  } });
  Object.assign(window, { settingsShellFixture: { open: () => setSettingsOpen(true) } });
  document.documentElement.dataset.fixtureRenders = String(++appRenders);
  return (
    <TooltipProvider>
      <div style={{ height: "100vh" }}>
        <MainLayout sidebar={
          <SessionSidebar projects={[{ root: "/repo", name: "Fixture project with a long workspace name",
            current: !workspacePanels || !officeMode, last_opened: 0, sessions: Array.from({ length: 40 }, (_, i) => ({
              id: i + 1, title: `Session ${i + 1} — long session label`, preview: "",
              active: i === 0, running: false, pinned: false, updated_at: 40 - i,
            })) }, ...(workspacePanels ? [{ root: "/office", name: "Fixture office", office: true,
              current: officeMode, last_opened: 0, sessions: [] }] : [])]}
            projectsLoaded activeId={1} hasWorkspace onNew={() => {}} onOpenSession={() => {}}
            onSwitchMode={root => { if (workspacePanels) setOfficeMode(root === "/office"); }}
            onToggleTerminal={useLayoutStore.getState().toggleTerminal} />
        } terminal={<TerminalDock cwd="/repo" />}>
        <Profiler id="chat" onRender={() => {
          const metrics = (window as unknown as { __terminalMetrics?: { start: number; chatCommits?: number[] } }).__terminalMetrics;
          metrics?.chatCommits?.push(performance.now() - metrics.start);
        }}>
          <ChatPage title="Terminal layout regression — 长标题与独立工作面" officeMode={officeMode} workspaceRoot={welcomeScene === "unrooted" || new URLSearchParams(location.search).has("welcome") ? "" : workspaceRoot} sessionsEmpty={["workspace", "office"].includes(welcomeScene) || new URLSearchParams(location.search).has("empty")} sessionId={sessionId} sessionKey={`${workspaceRoot}:${sessionId}`}
            recents={welcomeResponsive ? [{ path: "/recent", name: "Recent fixture", last_opened: 0 }] : undefined}
            onOpenWorkspace={() => { welcomeClicks.open++; }}
            onNewSession={() => { welcomeClicks.newSession++; }}
            onOpenRecent={() => { welcomeClicks.recent++; }} />
        </Profiler>
        </MainLayout>
        {settingsOpen && <div className="fixed inset-0 z-40"><SettingsPage onClose={() => setSettingsOpen(false)} /></div>}
      </div>
    </TooltipProvider>
  );
}
const params = new URLSearchParams(location.search);
createRoot(document.getElementById("root")!).render(
  params.has("appearance") ? (
    <TooltipProvider><Suspense fallback={null}><AppearanceFixture /></Suspense></TooltipProvider>
  ) : params.has("settings") ? (
    <TooltipProvider>
      <div data-terminal-settings-fixture style={{ height: "100vh", overflow: "auto", padding: 24 }}>
        <Suspense fallback={null}><TerminalSettings /></Suspense>
      </div>
    </TooltipProvider>
  ) : <App />,
);
