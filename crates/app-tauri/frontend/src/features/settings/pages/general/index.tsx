// 「通用」pane — the default permission / thinking / agent-mode / sandbox
// cards. Owns the prefs it edits; the shell only routes to it.

import { useEffect, useState } from "react";
import {
  Activity,
  Archive,
  Bot,
  Cpu,
  Globe,
  ShieldCheck,
  Sparkles,
  SquarePen,
  TriangleAlert,
  Zap,
} from "@keyline-icons/react";
import { Badge } from "@/components/ui/badge";
import { SettingsRenderer, type SettingsCardOption } from "@/features/settings/components";
import { getDefaultPrefs, setDefaultPrefs, getSandboxInfo, type SandboxInfo } from "@/lib/agent-ipc/sessions";

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
        className="text-[10px] px-1.5 py-0 font-medium h-4 bg-emerald-600 hover:bg-emerald-600 text-zinc-50"
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

export function GeneralPane() {
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

  const save = (patch: Parameters<typeof setDefaultPrefs>[0]) =>
    setDefaultPrefs(patch).catch((e) => console.error("save prefs failed:", e));

  return (
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
                        description:
                          "历史占用达到上下文窗口的该比例时触发自动压缩",
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
  );
}
