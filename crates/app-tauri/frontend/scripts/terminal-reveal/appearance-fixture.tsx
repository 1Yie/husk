import { CircleCheck, FileText } from "lucide-react";
import { PillIsland, type IslandPill } from "@/features/chat/components/pill-island";
import { SessionSidebar } from "@/features/chat/components/session-sidebar";
import { applyAppearance } from "@/lib/appearance";
import type { ProjectOverview, ProjectSessionRow } from "@/types";

const row = (id: number, title: string, extra: Partial<ProjectSessionRow> = {}): ProjectSessionRow => ({
  id, title, preview: "", active: id === 1, running: false, pinned: false, updated_at: 1000 - id, ...extra,
});
const projects: ProjectOverview[] = [
  { root: "/repo", name: "终端项目", current: true, last_opened: 1000, sessions: [
    row(1, "正在查看的会话"), row(2, "最近完成的会话"), row(3, "已归档的会话", { archived: true }),
  ] },
  { root: "/notes", name: "资料项目", current: false, last_opened: 900, sessions: [row(4, "跨项目任务")] },
  { root: "/office", name: "工作", current: false, office: true, last_opened: 800, sessions: [] },
];
const UI_FONT = '"Inter", "MiSans", system-ui, sans-serif';
function setMode(mode: "light" | "dark", font = UI_FONT) {
  applyAppearance({
    theme_mode: mode, accent: "#339cff", background: "#ffffff", foreground: "#1a1c1f",
    ui_font: font, code_font: '"JetBrains Mono", monospace', contrast: 45,
  });
}
declare global {
  interface Window {
    __appearanceFixture: { setMode: typeof setMode };
  }
}
window.__appearanceFixture = { setMode };
setMode("light");

const pills: IslandPill[] = [
  { key: "dock", label: "侧栏", tooltip: "打开侧栏", icon: <FileText size={16} />, text: <>
    <span className="font-mono tabular-nums text-emerald-600 dark:text-emerald-400">+14002</span>
    <span className="font-mono tabular-nums text-rose-600 dark:text-rose-400">−640</span>
  </> },
  { key: "jobs", label: "后台任务", tooltip: "4 个任务已结束",
    icon: <CircleCheck size={14} className="text-emerald-600 dark:text-emerald-400" />,
    text: <span data-appearance-text="jobs" className="flex-none whitespace-nowrap">4 个任务已结束</span> },
  { key: "subagents", label: "子代理", tooltip: "2 个子代理已结束",
    icon: <CircleCheck size={14} className="text-emerald-600 dark:text-emerald-400" />,
    text: <span data-appearance-text="subagents" className="flex-none whitespace-nowrap">2 个子代理已结束</span> },
];

export default function AppearanceFixture() {
  return (
    <div data-appearance-fixture style={{ display: "flex", height: "100vh" }}>
      <div data-appearance-sidebar>
        <SessionSidebar projects={projects} activeId={1} version="0.0.0"
          onNew={() => {}} onOpenSession={() => {}} onSwitchMode={() => {}}
          onOpenSettings={() => {}} onToggleTerminal={() => {}} />
      </div>
      <main style={{ flex: 1, minWidth: 0, display: "flex", justifyContent: "center", alignItems: "flex-end", paddingBottom: 112 }}>
        <PillIsland pills={pills} />
      </main>
    </div>
  );
}
