// Settings page — full-window takeover inside the main window (Zed-style):
// clicking the sidebar gear swaps the chat layout for a settings view with
// its own left nav + scrollable content, same window, same session running
// underneath. Esc or "返回工作区" goes back.

import { useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  Bot,
  Brain,
  Cpu,
  FileText,
  FlaskConical,
  Info,
  ListCheck,
  Palette,
  Plug,
  SlidersHorizontal,
  Sparkles,
  Terminal,
  Wrench,
  ChartColumn,
  Keyboard,
} from "@keyline-icons/react";
import { cn } from "@/lib/utils";
import { WindowControls } from "@/components/window-controls";
import { framedShell, isMac } from "@/lib/platform";
import { GeneralPane } from "@/features/settings/pages/general/index";
import { DeveloperPane } from "@/features/settings/pages/general/developer";
import { AppearanceSettings } from "@/features/settings/pages/appearance/index";
import { TerminalSettings } from "@/features/settings/pages/terminal/index";
import { ShortcutsSettings } from "@/features/settings/pages/shortcuts/index";
import { StatsPane } from "@/features/settings/pages/stats/index";
import { AboutSettings } from "@/features/settings/pages/about/index";
import {
  AgentSettings,
  type AgentTab,
} from "@/features/settings/pages/agent/index";
import { getDevConfig } from "@/lib/agent-ipc/sessions";

type SettingsTab =
  | "general"
  | "appearance"
  | "terminal"
  | "shortcuts"
  | "developer"
  | "stats"
  | AgentTab
  | "about";

const NAV_GROUPS: {
  label: string;
  items: { key: SettingsTab; label: string; icon: React.ReactNode }[];
}[] = [
  {
    label: "基础设置",
    items: [
      {
        key: "general",
        label: "常规",
        icon: <SlidersHorizontal className="h-4 w-4 shrink-0" />,
      },
      {
        key: "appearance",
        label: "外观",
        icon: <Palette className="h-4 w-4 shrink-0" />,
      },
      {
        key: "terminal",
        label: "终端",
        icon: <Terminal className="h-4 w-4 shrink-0" />,
      },
      {
        key: "shortcuts",
        label: "快捷键",
        icon: <Keyboard className="h-4 w-4 shrink-0" />,
      },
    ],
  },
  {
    label: "智能体",
    items: [
      {
        key: "instructions",
        label: "指令",
        icon: <FileText className="h-4 w-4 shrink-0" />,
      },
      {
        key: "rules",
        label: "规则",
        icon: <ListCheck className="h-4 w-4 shrink-0" />,
      },
      {
        key: "model",
        label: "模型",
        icon: <Cpu className="h-4 w-4 shrink-0" />,
      },
      {
        key: "memory",
        label: "记忆",
        icon: <Brain className="h-4 w-4 shrink-0" />,
      },
      {
        key: "skills",
        label: "技能",
        icon: <Sparkles className="h-4 w-4 shrink-0" />,
      },
      {
        key: "plugins",
        label: "插件",
        icon: <Plug className="h-4 w-4 shrink-0" />,
      },
      {
        key: "mcp",
        label: "连接器",
        icon: <Wrench className="h-4 w-4 shrink-0" />,
      },
      {
        key: "subagent",
        label: "子代理",
        icon: <Bot className="h-4 w-4 shrink-0" />,
      },
    ],
  },
  {
    label: "系统",
    items: [
      {
        key: "stats",
        label: "统计",
        icon: <ChartColumn className="h-4 w-4 shrink-0" />,
      },
      {
        key: "developer",
        label: "开发者选项",
        icon: <FlaskConical className="h-4 w-4 shrink-0" />,
      },
      {
        key: "about",
        label: "关于",
        icon: <Info className="h-4 w-4 shrink-0" />,
      },
    ],
  },
];

export function SettingsPage({
  onClose,
  initialTab = "general",
}: {
  onClose: () => void;
  /** Where the overlay opens — a command can ask for 技能 / 记忆 directly. The
   *  shell mounts this fresh on every open, so this is a starting point, not a
   *  controlled value. */
  initialTab?: SettingsTab;
}) {
  const [activeTab, setActiveTab] = useState<SettingsTab>(initialTab);
  const [devUnlocked, setDevUnlocked] = useState(false);
  const contentRef = useRef<HTMLDivElement>(null);
  // Esc goes back to the workspace — same reflex as every full-screen pane.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Read the persisted unlock flag once on mount. The dev pane is hidden
  // by default — opting in is a manual edit of
  // `~/.config/husk/settings.toml` (`developer_ui = true`), so a reload is
  // always required anyway; no event subscription needed.
  useEffect(() => {
    void getDevConfig()
      .then((c) => setDevUnlocked(c.developer_ui === true))
      .catch(() => {});
  }, []);

  // Each tab opens at the top — the container keeps its scrollTop otherwise.
  useEffect(() => {
    contentRef.current?.scrollTo({ top: 0 });
  }, [activeTab]);

  const tabMeta = NAV_GROUPS.flatMap((g) => g.items).find(
    (i) => i.key === activeTab,
  );

  return (
    <div className={cn(
      "settings-layout relative flex h-full w-full overflow-hidden select-none",
      framedShell ? "main-workspace-frame bg-panel" : "bg-white",
    )}>
      {/* Top draggable strip across the right area + window controls anchored at top-right */}
      <div
        data-tauri-drag-region="deep"
        className="absolute top-0 left-[240px] right-0 z-30 flex h-9 items-center justify-end px-3 select-none"
      >
        <WindowControls />
      </div>

      {/* Left nav — same rail as session sidebar: panel bg, white active card */}
      <aside
        data-tauri-drag-region="deep"
        className={cn(
          "w-[240px] flex-none flex flex-col justify-between h-full select-none",
          framedShell ? "bg-transparent" : "bg-panel border-r border-hairline",
        )}
      >
        <div className="flex flex-col min-h-0 flex-1">
          {/* Top title strip matching the main sidebar */}
          <div
            data-tauri-drag-region="deep"
            className={cn(
              "h-9 flex-none flex items-center px-3 gap-2 select-none cursor-default",
              framedShell ? "bg-transparent" : "bg-panel",
            )}
          >
            {/* The traffic lights end near 72px; this leaves a small gap before the title. */}
            {isMac && <div className="w-[68px] shrink-0" />}
            <span className="text-[12px] font-semibold text-neutral-700 tracking-tight">
              设置
            </span>
          </div>

          <div className="flex-none px-3 pt-3 flex flex-col">
            <button
              type="button"
              onClick={onClose}
              className="flex items-center gap-2 px-2 h-8 rounded-lg text-[13px] font-medium text-neutral-500 hover:text-neutral-800 hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)] transition-colors cursor-pointer"
            >
              <ArrowLeft className="h-4 w-4 shrink-0" />
              返回工作区
            </button>
          </div>

          {/* Same scroller rules as the session sidebar — `false` outer so a
              scrollbar drag can't move the window, `deep` inner so row gaps and
              the empty tail still drag it. */}
          <div
            data-settings-nav
            data-tauri-drag-region="false"
            className="min-h-0 flex-1 overflow-y-auto px-3 pt-3 pb-3"
          >
            <div data-tauri-drag-region="deep" className="min-h-full flex flex-col gap-4">
              {NAV_GROUPS.map((g) => (
                <div key={g.label} className="flex flex-col gap-1">
                  <span className="px-3 pb-1 text-[11px] font-medium text-neutral-500 tracking-wide">
                    {g.label}
                  </span>
                  {g.items
                    .filter((item) => item.key !== "developer" || devUnlocked)
                    .map((item) => (
                      <button
                        key={item.key}
                        type="button"
                        onClick={() => setActiveTab(item.key)}
                        className={cn(
                          "w-full flex items-center gap-2.5 h-9 rounded-lg px-3 text-[13px] transition-all cursor-pointer",
                          activeTab === item.key
                            ? // Same recipe as the session sidebar's active row:
                              // 6% black wash + bright text — the vars lift it in
                              // both themes, no dark: override needed.
                              "bg-[color-mix(in_srgb,var(--husk-black)_6%,transparent)] text-neutral-900"
                            : "text-neutral-600 hover:text-neutral-900 hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)] font-normal",
                        )}
                      >
                        {item.icon}
                        <span>{item.label}</span>
                      </button>
                    ))}
                </div>
              ))}
            </div>
          </div>
        </div>
      </aside>

      {/* Content — big page title + grouped setting rows. */}
      <div
        ref={contentRef}
        className={cn(
          "flex-1 min-h-0 min-w-0 overflow-y-auto bg-white no-scrollbar",
          framedShell && "m-2 rounded-lg border border-hairline",
        )}
      >
        <div className="w-full max-w-3xl mx-auto px-6 sm:px-10 py-8 pb-16">
          <h1 className="text-xl font-semibold text-neutral-900 mb-6 tracking-tight">
            {tabMeta?.label}
          </h1>

          {activeTab === "general" ? (
            <GeneralPane />
          ) : activeTab === "stats" ? (
            <StatsPane />
          ) : activeTab === "developer" ? (
            <DeveloperPane />
          ) : activeTab === "appearance" ? (
            <AppearanceSettings />
          ) : activeTab === "terminal" ? (
            <TerminalSettings />
          ) : activeTab === "shortcuts" ? (
            <ShortcutsSettings />
          ) : activeTab === "about" ? (
            <AboutSettings />
          ) : (
            <AgentSettings tab={activeTab as AgentTab} />
          )}
        </div>
      </div>
    </div>
  );
}
