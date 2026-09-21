// Settings page — full-window takeover inside the main window (Zed-style):
// clicking the sidebar gear swaps the chat layout for a settings view with
// its own left nav + scrollable content, same window, same session running
// underneath. Esc or "返回工作区" goes back.

import { useEffect, useState } from "react";
import {
  SlidersHorizontal,
  TriangleAlert,
  Zap,
  SquarePen,
  ShieldCheck,
  Sparkles,
  Palette,
  ArrowLeft,
  Bot,
} from "@keyline-icons/react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { TitleBar } from "@/components/title-bar";
import { cn } from "@/lib/utils";
import { getDefaultPrefs, setDefaultPrefs } from "../../invoke/agent/sessions";
import { AppearanceSettings } from "./appearance";
import {
  SettingsRenderer,
  type SettingsCardOption,
} from "@/components/settings";

const PERMISSION_OPTIONS: SettingsCardOption[] = [
  {
    value: "auto",
    label: "自动",
    badge: (
      <Badge
        variant="secondary"
        className="text-[10px] px-1.5 py-0 font-medium h-4"
      >
        推荐
      </Badge>
    ),
    description: "自动执行文件修改与常规命令，仅在遇到高危指令（如删除、外网访问）时弹窗确认。",
    icon: <Zap className="h-4 w-4 text-amber-500 dark:text-amber-400 shrink-0" />,
  },
  {
    value: "bypassPermissions",
    label: "全部权限",
    badge: (
      <Badge
        variant="default"
        className="text-[10px] px-1.5 py-0 font-medium h-4 bg-emerald-600 hover:bg-emerald-600 text-white dark:text-[#fafafa]"
      >
        完全信任
      </Badge>
    ),
    description: "全自动自主执行，跳过所有修改和命令的确认弹窗，实现最高执行效率。",
    icon: <TriangleAlert className="h-4 w-4 text-emerald-500 dark:text-emerald-400 shrink-0" />,
  },
  {
    value: "acceptEdits",
    label: "仅接受编辑",
    description: "自动允许代码与文件修改，但在执行终端命令前仍需手动批准。",
    icon: <SquarePen className="h-4 w-4 text-blue-500 dark:text-blue-400 shrink-0" />,
  },
  {
    value: "default",
    label: "每次确认",
    description: "最严格的安全模式，凡是涉及修改文件或运行终端命令前均需手动批准。",
    icon: <ShieldCheck className="h-4 w-4 text-neutral-500 shrink-0" />,
  },
];

const THINKING_OPTIONS = [
  { value: "off", label: "关闭思考", desc: "不使用额外推理过程，响应最快" },
  { value: "low", label: "轻度思考", desc: "进行快速简短的分析" },
  { value: "medium", label: "中等思考", desc: "平衡速度与分析质量" },
  { value: "high", label: "深度思考", desc: "深层推演架构与复杂逻辑" },
  { value: "max", label: "最大推演", desc: "全速深度推理，解决高难任务" },
];

const AGENT_MODE_OPTIONS = [
  { value: "build", label: "构建", desc: "完整工具集 — 读写、执行、验证" },
  { value: "plan", label: "计划", desc: "只读分析，产出实施方案，批准后执行" },
  { value: "goal", label: "目标", desc: "自主推进直到目标达成或明确受阻" },
];

type SettingsTab = "general" | "appearance";

const NAV_GROUPS: { label: string; items: { key: SettingsTab; label: string; icon: React.ReactNode }[] }[] = [
  {
    label: "基础设置",
    items: [
      { key: "general", label: "常规", icon: <SlidersHorizontal className="h-4 w-4 shrink-0" /> },
      { key: "appearance", label: "外观", icon: <Palette className="h-4 w-4 shrink-0" /> },
    ],
  },
];

export function SettingsPage({ onClose }: { onClose: () => void }) {
  const [activeTab, setActiveTab] = useState<SettingsTab>("general");
  const [permission, setPermission] = useState("auto");
  const [thinking, setThinking] = useState("medium");
  const [agentMode, setAgentMode] = useState("build");

  useEffect(() => {
    void (async () => {
      try {
        const prefs = await getDefaultPrefs();
        if (prefs.permission_mode) setPermission(prefs.permission_mode);
        if (prefs.thinking_level) setThinking(prefs.thinking_level);
        if (prefs.agent_mode) setAgentMode(prefs.agent_mode);
      } catch (e) {
        console.error("load settings error:", e);
      }
    })();
  }, []);

  // Esc goes back to the workspace — same reflex as every full-screen pane.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const save = (patch: Parameters<typeof setDefaultPrefs>[0]) =>
    setDefaultPrefs(patch).catch((e) => console.error("save prefs failed:", e));

  const tabMeta = NAV_GROUPS.flatMap((g) => g.items).find((i) => i.key === activeTab);

  return (
    <div className="flex h-full w-full flex-col bg-white overflow-hidden select-none">
      <TitleBar title="设置" />
      <div className="flex flex-1 overflow-hidden">
        {/* Left nav — same rail as the session sidebar: panel bg, white active
            card, h-8-ish rows. The back affordance sits on top like the
            reference's "返回工作区". */}
        <aside className="w-56 flex-none bg-panel border-r border-hairline flex flex-col p-3 select-none justify-between">
          <div className="flex flex-col gap-4">
            <button
              type="button"
              onClick={onClose}
              className="flex items-center gap-2 px-2 h-8 rounded-lg text-[13px] font-medium text-neutral-600 hover:text-neutral-900 hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)] transition-colors cursor-pointer"
            >
              <ArrowLeft className="h-4 w-4 shrink-0" />
              返回工作区
            </button>

            {NAV_GROUPS.map((g) => (
              <div key={g.label} className="flex flex-col gap-1">
                <span className="px-3 pb-1 text-[11px] font-medium text-neutral-400 tracking-wide">
                  {g.label}
                </span>
                {g.items.map((item) => (
                  <button
                    key={item.key}
                    type="button"
                    onClick={() => setActiveTab(item.key)}
                    className={cn(
                      "w-full flex items-center gap-2.5 h-9 rounded-lg px-3 text-[13px] transition-all cursor-pointer",
                      activeTab === item.key
                        ? "bg-white text-neutral-900 font-semibold shadow-xs border border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)]"
                        : "text-neutral-600 hover:text-neutral-900 hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)] font-medium"
                    )}
                  >
                    {item.icon}
                    <span>{item.label}</span>
                  </button>
                ))}
              </div>
            ))}
          </div>

          <span className="text-[11px] text-neutral-400 font-mono block text-center">
            Husk v0.1.0
          </span>
        </aside>

        {/* Content — big page title + grouped setting rows. */}
        <div className="flex-1 min-h-0 overflow-y-auto bg-white">
          <div className="px-10 py-8 max-w-3xl">
            <h1 className="text-xl font-semibold text-neutral-900 mb-6">
              {tabMeta?.label}
            </h1>

            {activeTab === "general" ? (
              <SettingsRenderer
                sections={[
                  {
                    kind: "cards",
                    key: "permission",
                    title: "默认权限模式",
                    value: permission,
                    onChange: (v) => {
                      setPermission(v);
                      void save({ permission_mode: v });
                    },
                    options: PERMISSION_OPTIONS,
                  },
                  {
                    kind: "list",
                    key: "agent",
                    title: "Agent 偏好",
                    fields: [
                      {
                        key: "agentMode",
                        type: "select",
                        label: "默认代理模式",
                        description: "新会话启动时的代理模式；运行中可在输入框随时切换",
                        icon: <Bot className="h-4 w-4 text-neutral-500" />,
                        value: agentMode,
                        onChange: (v) => {
                          setAgentMode(v);
                          void save({ agent_mode: v });
                        },
                        placeholder: "选择代理模式",
                        options: AGENT_MODE_OPTIONS.map((t) => ({
                          value: t.value,
                          label: t.label,
                          description: t.desc,
                        })),
                      },
                      {
                        key: "thinking",
                        type: "select",
                        label: "默认思考深度",
                        description: "新会话启动时的默认推理思考深度",
                        icon: <Sparkles className="h-4 w-4 text-neutral-500" />,
                        value: thinking,
                        onChange: (v) => {
                          setThinking(v);
                          void save({ thinking_level: v });
                        },
                        placeholder: "选择思考深度",
                        options: THINKING_OPTIONS.map((t) => ({
                          value: t.value,
                          label: t.label,
                          description: t.desc,
                        })),
                      },
                    ],
                  },
                ]}
              />
            ) : (
              <AppearanceSettings />
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
