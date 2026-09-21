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
  Archive,
  Globe,
  Cpu,
  Activity,
  Info,
  FileText,
  Wrench,
} from "@keyline-icons/react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { WindowControls } from "@/components/window-controls";
import { isMac } from "@/lib/platform";
import { cn } from "@/lib/utils";
import { getDefaultPrefs, setDefaultPrefs, getSandboxInfo, type SandboxInfo } from "../../invoke/agent/sessions";
import { AppearanceSettings } from "./appearance";
import { AboutSettings } from "./about";
import { AgentSettings, type AgentTab } from "./agent";
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
  { value: "off", label: "关闭思考（off）", desc: "不使用额外推理过程，响应最快" },
  { value: "minimal", label: "最小推理（minimal）", desc: "最轻量推理深度" },
  { value: "low", label: "轻度思考（low）", desc: "进行快速简短的分析" },
  { value: "medium", label: "中等思考（medium）", desc: "平衡速度与分析质量" },
  { value: "high", label: "深度思考（high）", desc: "深层推演架构与复杂逻辑" },
  { value: "xhigh", label: "超高思考（xhigh）", desc: "超深层推演，慢而透彻" },
  { value: "max", label: "最大推理（max）", desc: "全速深度推理，解决高难任务" },
];

const AGENT_MODE_OPTIONS = [
  { value: "build", label: "构建", desc: "完整工具集 — 读写、执行、验证" },
  { value: "plan", label: "计划", desc: "只读分析，产出实施方案，批准后执行" },
  { value: "goal", label: "目标", desc: "自主推进直到目标达成或明确受阻" },
];

const COMPACT_AT_OPTIONS = [
  { value: "70", label: "70%", desc: "更早压缩，给后续轮次留足余量" },
  { value: "80", label: "80%", desc: "默认平衡值" },
  { value: "90", label: "90%", desc: "尽量保留原始上下文，压缩更晚" },
];

const SANDBOX_NETWORK_OPTIONS = [
  { value: "auto", label: "自动", desc: "按命令审计结果放行网络（默认）" },
  { value: "allow", label: "始终允许", desc: "所有沙盒命令均可联网" },
  { value: "deny", label: "始终禁止", desc: "强制断开所有沙盒命令的网络" },
];

const SANDBOX_MEMORY_OPTIONS = [
  { value: "default", label: "默认（2048 MB）" },
  { value: "512", label: "512 MB" },
  { value: "1024", label: "1 GB" },
  { value: "4096", label: "4 GB" },
  { value: "8192", label: "8 GB" },
];

const SANDBOX_PROCS_OPTIONS = [
  { value: "default", label: "默认（256）" },
  { value: "64", label: "64" },
  { value: "128", label: "128" },
  { value: "512", label: "512" },
  { value: "1024", label: "1024" },
];

type SettingsTab = "general" | "appearance" | AgentTab | "about";

const NAV_GROUPS: { label: string; items: { key: SettingsTab; label: string; icon: React.ReactNode }[] }[] = [
  {
    label: "基础设置",
    items: [
      { key: "general", label: "常规", icon: <SlidersHorizontal className="h-4 w-4 shrink-0" /> },
      { key: "appearance", label: "外观", icon: <Palette className="h-4 w-4 shrink-0" /> },
    ],
  },
  {
    label: "智能体",
    items: [
      { key: "instructions", label: "指令", icon: <FileText className="h-4 w-4 shrink-0" /> },
      { key: "model", label: "模型", icon: <Cpu className="h-4 w-4 shrink-0" /> },
      { key: "skills", label: "技能", icon: <Sparkles className="h-4 w-4 shrink-0" /> },
      { key: "mcp", label: "MCP", icon: <Wrench className="h-4 w-4 shrink-0" /> },
      { key: "subagent", label: "SubAgent", icon: <Bot className="h-4 w-4 shrink-0" /> },
    ],
  },
  {
    label: "系统",
    items: [
      { key: "about", label: "关于", icon: <Info className="h-4 w-4 shrink-0" /> },
    ],
  },
];

export function SettingsPage({ onClose }: { onClose: () => void }) {
  const [activeTab, setActiveTab] = useState<SettingsTab>("general");
  const [permission, setPermission] = useState("auto");
  const [thinking, setThinking] = useState("medium");
  const [agentMode, setAgentMode] = useState("build");
  const [compactAt, setCompactAt] = useState("80");
  const [sandboxNetwork, setSandboxNetwork] = useState("auto");
  const [sandboxMem, setSandboxMem] = useState("default");
  const [sandboxProcs, setSandboxProcs] = useState("default");
  const [sandboxInfo, setSandboxInfo] = useState<SandboxInfo | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        const prefs = await getDefaultPrefs();
        if (prefs.permission_mode) setPermission(prefs.permission_mode);
        if (prefs.thinking_level) setThinking(prefs.thinking_level);
        if (prefs.agent_mode) setAgentMode(prefs.agent_mode);
        if (prefs.compact_at) setCompactAt(String(Math.round(prefs.compact_at * 100)));
        if (prefs.sandbox_network) setSandboxNetwork(prefs.sandbox_network);
        setSandboxMem(prefs.sandbox_max_memory_mb ? String(prefs.sandbox_max_memory_mb) : "default");
        setSandboxProcs(prefs.sandbox_max_processes ? String(prefs.sandbox_max_processes) : "default");
        void getSandboxInfo()
          .then(setSandboxInfo)
          .catch(() => {});
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
    <div className="relative flex h-full w-full bg-white overflow-hidden select-none">
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
        className="w-[240px] flex-none bg-panel border-r border-hairline flex flex-col justify-between h-full select-none"
      >
        <div className="flex flex-col">
          {/* Top title strip matching the main sidebar */}
          <div
            data-tauri-drag-region="deep"
            className="h-9 flex-none flex items-center px-3 gap-2 bg-panel select-none cursor-default"
          >
            {isMac && <div className="w-[78px] shrink-0" />}
            <span className="text-[12px] font-semibold text-neutral-700 tracking-tight">
              设置
            </span>
          </div>

          <div className="p-3 flex flex-col gap-4">
            <button
              type="button"
              onClick={onClose}
              className="flex items-center gap-2 px-2 h-8 rounded-lg text-[13px] font-medium text-neutral-500 hover:text-neutral-800 hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)] transition-colors cursor-pointer"
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
                        ? // Same recipe as the session sidebar's active row:
                          // 6% black wash + bright text — the vars lift it in
                          // both themes, no dark: override needed.
                          "bg-[color-mix(in_srgb,var(--husk-black)_6%,transparent)] text-neutral-900"
                        : "text-neutral-600 hover:text-neutral-900 hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)] font-normal"
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

      </aside>

        {/* Content — big page title + grouped setting rows. */}
        <div className="flex-1 min-h-0 overflow-y-auto bg-white no-scrollbar">
          <div className="w-full max-w-3xl mx-auto px-6 sm:px-10 py-8 pb-16">
            <h1 className="text-xl font-semibold text-neutral-900 mb-6 tracking-tight">
              {tabMeta?.label}
            </h1>

            {activeTab === "general" ? (
              <SettingsRenderer
                sections={[
                  {
                    kind: "cards",
                    key: "permission",
                    title: "默认权限模式",
                    description: "配置 Agent 执行文件修改与终端命令时的默认权限拦截级别",
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
                    description: "配置新会话启动时的默认运行模式与推理思考深度",
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
                      {
                        key: "compactAt",
                        type: "select",
                        label: "上下文压缩阈值",
                        description: "历史占用达到上下文窗口的该比例时触发自动压缩",
                        icon: <Archive className="h-4 w-4 text-neutral-500" />,
                        value: compactAt,
                        onChange: (v) => {
                          setCompactAt(v);
                          void save({ compact_at: Number(v) / 100 });
                        },
                        placeholder: "选择阈值",
                        options: COMPACT_AT_OPTIONS.map((t) => ({
                          value: t.value,
                          label: t.label,
                          description: t.desc,
                        })),
                      },
                    ],
                  },
                  {
                    kind: "list",
                    key: "sandbox",
                    title: "沙盒设置",
                    description: `命令执行的隔离与资源限制${
                      sandboxInfo ? `（当前后端：${sandboxInfo.backend} · ${sandboxInfo.tier}）` : ""
                    }`,
                    fields: [
                      {
                        key: "sandboxNetwork",
                        type: "select",
                        label: "网络访问",
                        description: "沙盒内命令的网络放行策略",
                        icon: <Globe className="h-4 w-4 text-neutral-500" />,
                        value: sandboxNetwork,
                        onChange: (v) => {
                          setSandboxNetwork(v);
                          void save({ sandbox_network: v as "auto" | "allow" | "deny" });
                        },
                        placeholder: "选择网络策略",
                        options: SANDBOX_NETWORK_OPTIONS.map((t) => ({
                          value: t.value,
                          label: t.label,
                          description: t.desc,
                        })),
                      },
                      {
                        key: "sandboxMem",
                        type: "select",
                        label: "内存上限",
                        description: "单条命令的常驻内存上限，超出即终止",
                        icon: <Cpu className="h-4 w-4 text-neutral-500" />,
                        value: sandboxMem,
                        onChange: (v) => {
                          setSandboxMem(v);
                          void save({ sandbox_max_memory_mb: v === "default" ? null : Number(v) });
                        },
                        placeholder: "选择内存上限",
                        options: SANDBOX_MEMORY_OPTIONS,
                      },
                      {
                        key: "sandboxProcs",
                        type: "select",
                        label: "进程数上限",
                        description: "单条命令可派生的最大进程数，防 fork 炸弹",
                        icon: <Activity className="h-4 w-4 text-neutral-500" />,
                        value: sandboxProcs,
                        onChange: (v) => {
                          setSandboxProcs(v);
                          void save({ sandbox_max_processes: v === "default" ? null : Number(v) });
                        },
                        placeholder: "选择进程数上限",
                        options: SANDBOX_PROCS_OPTIONS,
                      },
                    ],
                  },
                ]}
              />
            ) : activeTab === "appearance" ? (
              <AppearanceSettings />
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
