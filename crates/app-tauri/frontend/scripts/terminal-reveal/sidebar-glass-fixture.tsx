import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { MainLayout } from "@/app/layout/main-layout";
import { useLayoutStore } from "@/app/layout/layout-store";
import { SessionSidebar } from "@/features/chat/components/session-sidebar";
import { WorkspaceLoadingVeil } from "@/features/chat/components/workspace-loading-veil";
import { TooltipProvider } from "@/components/ui/tooltip";
import { applyAppearance } from "@/lib/appearance";
import { isMac } from "@/lib/platform";
import type { AppearanceConfig } from "@/lib/agent-ipc";
import "@/index.css";

const native = isTauri();
const metrics = { calls: [] as { at: number; end?: number; revision: number; surface: unknown; active: number }[],
  frames: [] as number[], active: 0, maxActive: 0 };
let mode = "active";
let delay = 0;
if (!native) {
  Object.assign(window, { isTauri: true });
  mockWindows("main");
  mockIPC(async (command, args) => {
    if (command === "sidebar_glass_update") {
      metrics.maxActive = Math.max(metrics.maxActive, ++metrics.active);
      const call = { at: performance.now(), revision: args.revision, surface: args.surface, active: metrics.active, end: undefined as number | undefined };
      metrics.calls.push(call);
      await new Promise((resolve) => setTimeout(resolve, delay));
      call.end = performance.now();
      metrics.active--;
      if (mode === "failure") throw new Error("fixture native failure");
      return mode === "active" && args.surface !== null;
    }
    if (command === "agent_session" && args.op === "get_appearance") return config;
    if (command === "agent_session") return [];
    if (command === "plugin:event|listen") return 1;
    return null;
  });
}
const config = {
  theme_mode: "light", accent: "#339cff", background: "#ffffff",
  foreground: "#1a1c1f", ui_font: "system-ui", code_font: "monospace", contrast: 45,
} as AppearanceConfig;
const rows = Array.from({ length: 80 }, (_, i) => ({
  id: i + 1, title: `Session ${String(i + 1).padStart(2, "0")} — 长标题与会话菜单验收`,
  preview: "", active: i === 0, running: false, pinned: false, updated_at: 80 - i,
}));
const projects = [{ root: "/fixture", name: "Fixture project", current: true, last_opened: 0, sessions: rows }];
useLayoutStore.setState({ sidebarOpen: true });
applyAppearance(config);
function Fixture() {
  const [selected, select] = useState(1);
  const [mounted, setMounted] = useState(true);
  const [loading, setLoading] = useState(false);
  Object.assign(window, { glassFixture: {
    metrics,
    native,
    setMode(value: string) { mode = value; window.dispatchEvent(new Event("husk-sidebar-glass-change")); },
    setDelay(value: number) { delay = value; },
    theme(value: "light" | "dark" | "custom") {
      applyAppearance({ ...config, theme_mode: value === "dark" ? "dark" : "light",
        background: value === "custom" ? "#e7dfcc" : "#ffffff",
        dark_background: "#16161a", ui_font: value === "custom" ? "Georgia" : "system-ui" });
    },
    mount(value: boolean) { setMounted(value); },
    loading: setLoading,
    toggle: useLayoutStore.getState().toggleSidebar,
    selected: () => selected,
    sample() {
      const times: number[] = [];
      return new Promise<number[]>((resolve) => {
        const start = performance.now();
        const tick = (now: number) => {
          times.push(now);
          if (now - start < 500) requestAnimationFrame(tick);
          else resolve(times.slice(1).map((v, i) => v - times[i]));
        };
        requestAnimationFrame(tick);
      });
    },
  } });
  return <TooltipProvider>{mounted && <MainLayout sidebar={<SessionSidebar
    projects={projects} projectsLoaded activeId={selected} hasWorkspace
    onNew={() => select(81)} onOpenSession={(_root, id) => select(id)}
    onSwitchMode={() => {}}
    actions={{ rename: () => {}, archive: () => {} }}
  />}>
    <div id="opaque-workspace-card" className={`flex-1 min-h-0 bg-workspace p-4 ${isMac ? "rounded-lg border border-hairline" : ""}`}>
      <h1>Sidebar glass integration</h1>
      <button id="toggle-sidebar" onClick={useLayoutStore.getState().toggleSidebar}>Toggle sidebar</button>
      <p id="selected-session">Selected session {selected}</p>
      <input id="main-input" aria-label="Main input" />
      <div className="bg-panel p-4" id="opaque-panel">Opaque main panel</div>
    </div>
  </MainLayout>}{loading && <WorkspaceLoadingVeil visible />}</TooltipProvider>;
}
createRoot(document.getElementById("root")!).render(<StrictMode><Fixture /></StrictMode>);
if (native) {
  const log = (value: unknown) => invoke("fixture_log", { value });
  window.addEventListener("error", (event) => { void log({ error: event.message }); });
  window.addEventListener("unhandledrejection", (event) => { void log({ rejection: String(event.reason) }); });
  void log({ fixture: "real MainLayout + SessionSidebar; real sidebar_glass_update IPC; fake in-memory sessions" });
}
