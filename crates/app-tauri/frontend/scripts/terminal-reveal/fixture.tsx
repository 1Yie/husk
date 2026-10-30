import { lazy, Profiler, Suspense } from "react";
import { createRoot } from "react-dom/client";
import "../../src/index.css";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useLayoutStore } from "@/app/layout/layout-store";
import { TerminalToggle } from "@/components/layout-controls";
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

const TerminalSettings = lazy(() => import("@/features/settings/pages/terminal")
  .then((module) => ({ default: module.TerminalSettings })));
const AppearanceFixture = lazy(() => import("./appearance-fixture"));

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
const view = viewFromHistory(history, null, 0);
useTerminalPrefs.setState(DEFAULT_TERMINAL_PREFS);
useLayoutStore.setState({ sidebarOpen: true, terminalOpen: false });
useAgentStore.getState().setWorkspaceRoot("/repo");
useAgentStore.getState().setActiveId(1);
useAgentStore.getState().loadView("/repo", 1, view);
useDockStore.setState({ open: true, tab: "tools", browserOpen: false, width: 300 });
let appRenders = 0;
function App() {
  document.documentElement.dataset.fixtureRenders = String(++appRenders);
  return (
    <TooltipProvider>
      <div style={{ height: "100vh" }}>
        <MainLayout sidebar={
          <div className="h-full flex flex-col justify-end p-3">
            <TerminalToggle onToggle={useLayoutStore.getState().toggleTerminal} />
          </div>
        } terminal={<TerminalDock cwd="/repo" />}>
        <Profiler id="chat" onRender={() => {
          const metrics = (window as unknown as { __terminalMetrics?: { start: number; chatCommits?: number[] } }).__terminalMetrics;
          metrics?.chatCommits?.push(performance.now() - metrics.start);
        }}>
          <ChatPage title="Terminal layout regression" workspaceRoot="/repo" sessionId={1} sessionKey="/repo:1" />
        </Profiler>
        </MainLayout>
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
