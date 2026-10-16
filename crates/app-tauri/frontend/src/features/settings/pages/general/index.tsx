// 「通用」pane — the default permission / thinking / agent-mode / sandbox
// cards. Owns the prefs it edits; the shell only routes to it.

import { useEffect, useMemo, useState } from "react";
import {
  Activity,
  Archive,
  Bell,
  Bot,
  Cpu,
  FileText,
  Globe,
  Layers,
  ShieldCheck,
  Sparkles,
  SquarePen,
  TriangleAlert,
  Zap,
} from "@keyline-icons/react";
import { Switch } from "@/components/ui/switch";
import { KvList, KvListContent } from "@/components/ui/kv-list";
import { SkeletonControl } from "@/components/ui/skeleton";
import {
  SettingSelect,
  SettingRow,
  SettingsCards,
  SettingsSection,
  Tag,
  type SettingsCardOption,
} from "@/features/settings/components";
import { THINKING_LEVELS } from "@/lib/thinking";
import {
  getDefaultPrefs,
  setDefaultPrefs,
  getAppConfig,
  type DefaultPrefs,
} from "@/lib/agent-ipc/sessions";

const PERMISSION_OPTIONS: SettingsCardOption[] = [
  {
    value: "auto",
    label: "自动",
    badge: <Tag>推荐</Tag>,
    description: "自动执行常规操作，高风险操作需要确认",
    icon: <Zap className="h-4 w-4" />,
    iconTone: "text-emerald-600 dark:text-emerald-300",
  },
  {
    value: "bypassPermissions",
    label: "全部权限",
    description: "文件修改和终端命令均自动执行",
    icon: <TriangleAlert className="h-4 w-4" />,
    iconTone: "text-rose-600 dark:text-rose-300",
  },
  {
    value: "acceptEdits",
    label: "仅接受编辑",
    description: "自动修改文件，终端命令需要确认",
    icon: <SquarePen className="h-4 w-4" />,
    iconTone: "text-sky-600 dark:text-sky-300",
  },
  {
    value: "default",
    label: "每次确认",
    description: "文件修改和终端命令均需确认",
    icon: <ShieldCheck className="h-4 w-4" />,
    iconTone: "text-violet-600 dark:text-violet-300",
  },
];

const THINKING_OPTIONS = THINKING_LEVELS.map((l) => ({ value: l.value, label: l.label }));

const AGENT_MODE_OPTIONS = [
  { value: "build", label: "构建" },
  { value: "plan", label: "计划" },
  { value: "goal", label: "目标" },
];

const WORKSPACE_MODE_OPTIONS = [
  { value: "last", label: "上次使用" },
  { value: "build", label: "编程" },
  { value: "office", label: "工作" },
];

const COMPACT_AT_OPTIONS = [
  { value: "70", label: "70%" },
  { value: "80", label: "80%" },
  { value: "90", label: "90%" },
];

const SANDBOX_NETWORK_OPTIONS = [
  { value: "auto", label: "自动" },
  { value: "allow", label: "始终允许" },
  { value: "deny", label: "始终禁止" },
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

/** Control values derived from stored prefs — absent fields fall back to
 * the same defaults the backend applies. */
function prefsToState(prefs: DefaultPrefs) {
  return {
    permission: prefs.permission_mode || "auto",
    activeModelKey:
      prefs.active_provider && prefs.active_model
        ? `${prefs.active_provider}/${prefs.active_model}`
        : "__config__",
    thinking: prefs.thinking_level || "medium",
    agentMode: prefs.agent_mode || "build",
    workspaceMode: prefs.default_workspace_mode || "last",
    compactAt: prefs.compact_at ? String(Math.round(prefs.compact_at * 100)) : "80",
    // absent = default-off — only an explicit true turns it on.
    sandboxEnabled: prefs.sandbox_enabled === true,
    blueprintEnabled: prefs.blueprint_enabled !== false,
    desktopNotify: prefs.desktop_notifications !== false,
    sandboxNetwork: (prefs.sandbox_network || "auto") as string,
    sandboxMem: prefs.sandbox_max_memory_mb ? String(prefs.sandbox_max_memory_mb) : "default",
    sandboxProcs: prefs.sandbox_max_processes ? String(prefs.sandbox_max_processes) : "default",
  };
}

/** Last loaded prefs + providers, kept across pane mounts so reopening the
 * pane renders real values at once instead of defaults that then jump. */
let cached: { prefs: DefaultPrefs; providers: Record<string, any> } | null = null;

export function GeneralPane() {
  const [init] = useState(() => prefsToState(cached?.prefs ?? {}));
  const [ready, setReady] = useState(cached !== null);
  const [permission, setPermission] = useState(init.permission);
  const [activeModelKey, setActiveModelKey] = useState(init.activeModelKey);
  const [thinking, setThinking] = useState(init.thinking);
  const [agentMode, setAgentMode] = useState(init.agentMode);
  const [workspaceMode, setWorkspaceMode] = useState(init.workspaceMode);
  const [compactAt, setCompactAt] = useState(init.compactAt);
  const [providers, setProviders] = useState<Record<string, any>>(cached?.providers ?? {});
  const [sandboxEnabled, setSandboxEnabled] = useState(init.sandboxEnabled);
  const [blueprintEnabled, setBlueprintEnabled] = useState(init.blueprintEnabled);
  const [desktopNotify, setDesktopNotify] = useState(init.desktopNotify);
  const [sandboxNetwork, setSandboxNetwork] = useState(init.sandboxNetwork);
  const [sandboxMem, setSandboxMem] = useState(init.sandboxMem);
  const [sandboxProcs, setSandboxProcs] = useState(init.sandboxProcs);

  useEffect(() => {
    let alive = true;
    // Both requests settle before any state is applied: prefs alone would
    // show the stored model as "已不存在" until the provider list arrives.
    void Promise.all([
      getDefaultPrefs(),
      getAppConfig()
        .then((r) => ((r.config as any)?.providers ?? {}) as Record<string, any>)
        .catch(() => cached?.providers ?? {}),
    ])
      .then(([prefs, provs]) => {
        cached = { prefs, providers: provs };
        if (!alive) return;
        const s = prefsToState(prefs);
        setPermission(s.permission);
        setActiveModelKey(s.activeModelKey);
        setThinking(s.thinking);
        setAgentMode(s.agentMode);
        setWorkspaceMode(s.workspaceMode);
        setCompactAt(s.compactAt);
        setSandboxEnabled(s.sandboxEnabled);
        setBlueprintEnabled(s.blueprintEnabled);
        setDesktopNotify(s.desktopNotify);
        setSandboxNetwork(s.sandboxNetwork);
        setSandboxMem(s.sandboxMem);
        setSandboxProcs(s.sandboxProcs);
        setProviders(provs);
      })
      .catch((e) => console.error("load settings error:", e))
      .finally(() => alive && setReady(true));
    return () => {
      alive = false;
    };
  }, []);

  const save = (patch: Parameters<typeof setDefaultPrefs>[0]) => {
    if (cached) cached = { ...cached, prefs: { ...cached.prefs, ...patch } };
    return setDefaultPrefs(patch).catch((e) => console.error("save prefs failed:", e));
  };

  // Flatten every provider's model list into one select — the value key
  // doubles as a lookup into `modelPairs` (model ids can contain `/`,
  // so never split the key back apart).
  const { modelOptions, modelPairs } = useMemo(() => {
    const opts: { value: string; label: string; description?: string }[] = [
      {
        value: "__config__",
        label: "跟随配置默认",
      },
    ];
    const pairs = new Map<string, { provider: string; model: string }>();
    for (const [pname, p] of Object.entries(providers)) {
      for (const raw of ((p as any)?.models ?? []) as any[]) {
        const id = typeof raw === "string" ? raw : raw?.id;
        if (!id) continue;
        const key = `${pname}/${id}`;
        const label = typeof raw === "object" && raw?.name ? raw.name : id;
        pairs.set(key, { provider: pname, model: id });
        opts.push({ value: key, label, description: pname });
      }
    }
    // A stored pair whose model was since deleted still needs a row so the
    // select doesn't render an empty value.
    if (!pairs.has(activeModelKey) && activeModelKey !== "__config__") {
      opts.push({ value: activeModelKey, label: activeModelKey, description: "已不存在" });
    }
    return { modelOptions: opts, modelPairs: pairs };
  }, [providers, activeModelKey]);

  return (
    <div className="flex w-full min-w-0 flex-col gap-8" aria-busy={!ready}>
      <SettingsSection title="默认权限模式" description="用于新建会话">
        <SettingsCards
          loading={!ready}
          value={permission}
          onChange={(v) => {
            setPermission(v);
            void save({ permission_mode: v });
          }}
          options={PERMISSION_OPTIONS}
        />
      </SettingsSection>

      <SettingsSection title="启动与会话">
        <KvList>
          <KvListContent>
            <SettingRow title="启动模式" icon={<FileText className="h-4 w-4" />} wideControl>
              {ready ? (
                <SettingSelect
                  value={workspaceMode}
                  onChange={(v) => {
                    setWorkspaceMode(v);
                    void save({ default_workspace_mode: v });
                  }}
                  options={WORKSPACE_MODE_OPTIONS}
                  ariaLabel="启动模式"
                />
              ) : (
                <SkeletonControl />
              )}
            </SettingRow>
            <SettingRow title="代理模式" icon={<Bot className="h-4 w-4" />} wideControl>
              {ready ? (
                <SettingSelect
                  value={agentMode}
                  onChange={(v) => {
                    setAgentMode(v);
                    void save({ agent_mode: v });
                  }}
                  options={AGENT_MODE_OPTIONS}
                  ariaLabel="代理模式"
                />
              ) : (
                <SkeletonControl />
              )}
            </SettingRow>
            <SettingRow title="默认模型" icon={<Cpu className="h-4 w-4" />} wideControl>
              {ready ? (
                <SettingSelect
                  value={activeModelKey}
                  onChange={(v) => {
                    setActiveModelKey(v);
                    const pair = modelPairs.get(v);
                    void save({
                      active_provider: pair?.provider ?? "",
                      active_model: pair?.model ?? "",
                    });
                  }}
                  options={modelOptions}
                  ariaLabel="默认模型"
                />
              ) : (
                <SkeletonControl />
              )}
            </SettingRow>
            <SettingRow title="思考深度" icon={<Sparkles className="h-4 w-4" />} wideControl>
              {ready ? (
                <SettingSelect
                  value={thinking}
                  onChange={(v) => {
                    setThinking(v);
                    void save({ thinking_level: v });
                  }}
                  options={THINKING_OPTIONS}
                  ariaLabel="思考深度"
                />
              ) : (
                <SkeletonControl />
              )}
            </SettingRow>
          </KvListContent>
        </KvList>
      </SettingsSection>

      <SettingsSection title="上下文">
        <KvList>
          <KvListContent>
            <SettingRow
              title="自动压缩"
              description="上下文达到此比例时压缩历史消息"
              icon={<Archive className="h-4 w-4" />}
              wideControl
            >
              {ready ? (
                <SettingSelect
                  value={compactAt}
                  onChange={(v) => {
                    setCompactAt(v);
                    void save({ compact_at: Number(v) / 100 });
                  }}
                  options={COMPACT_AT_OPTIONS}
                  ariaLabel="自动压缩阈值"
                />
              ) : (
                <SkeletonControl />
              )}
            </SettingRow>
            <SettingRow
              title="架构蓝图"
              description="向智能体提供项目的模块与依赖信息"
              icon={<Layers className="h-4 w-4" />}
            >
              {ready ? (
                <Switch
                  checked={blueprintEnabled}
                  aria-label="架构蓝图"
                  onCheckedChange={(v) => {
                    setBlueprintEnabled(v);
                    void save({ blueprint_enabled: v });
                  }}
                />
              ) : (
                <SkeletonControl kind="switch" />
              )}
            </SettingRow>
          </KvListContent>
        </KvList>
      </SettingsSection>

      <SettingsSection title="通知">
        <KvList>
          <KvListContent>
            <SettingRow
              title="系统通知"
              description="在桌面显示任务进度与待处理提醒"
              icon={<Bell className="h-4 w-4" />}
            >
              {ready ? (
                <Switch
                  checked={desktopNotify}
                  aria-label="系统通知"
                  onCheckedChange={(v) => {
                    setDesktopNotify(v);
                    void save({ desktop_notifications: v });
                  }}
                />
              ) : (
                <SkeletonControl kind="switch" />
              )}
            </SettingRow>
          </KvListContent>
        </KvList>
      </SettingsSection>

      <SettingsSection title="沙盒">
        <KvList>
          <KvListContent>
            <SettingRow
              title="启用沙盒"
              description="隔离命令执行环境"
              icon={<ShieldCheck className="h-4 w-4" />}
            >
              {ready ? (
                <Switch
                  checked={sandboxEnabled}
                  aria-label="启用沙盒"
                  onCheckedChange={(v) => {
                    setSandboxEnabled(v);
                    void save({ sandbox_enabled: v });
                  }}
                />
              ) : (
                <SkeletonControl kind="switch" />
              )}
            </SettingRow>
            <SettingRow title="网络访问" icon={<Globe className="h-4 w-4" />} wideControl>
              {ready ? (
                <SettingSelect
                  value={sandboxNetwork}
                  onChange={(v) => {
                    setSandboxNetwork(v);
                    void save({ sandbox_network: v as "auto" | "allow" | "deny" });
                  }}
                  options={SANDBOX_NETWORK_OPTIONS}
                  ariaLabel="沙盒网络访问"
                />
              ) : (
                <SkeletonControl />
              )}
            </SettingRow>
            <SettingRow title="内存上限" icon={<Cpu className="h-4 w-4" />} wideControl>
              {ready ? (
                <SettingSelect
                  value={sandboxMem}
                  onChange={(v) => {
                    setSandboxMem(v);
                    void save({ sandbox_max_memory_mb: v === "default" ? null : Number(v) });
                  }}
                  options={SANDBOX_MEMORY_OPTIONS}
                  ariaLabel="沙盒内存上限"
                />
              ) : (
                <SkeletonControl />
              )}
            </SettingRow>
            <SettingRow title="进程数上限" icon={<Activity className="h-4 w-4" />} wideControl>
              {ready ? (
                <SettingSelect
                  value={sandboxProcs}
                  onChange={(v) => {
                    setSandboxProcs(v);
                    void save({ sandbox_max_processes: v === "default" ? null : Number(v) });
                  }}
                  options={SANDBOX_PROCS_OPTIONS}
                  ariaLabel="沙盒进程数上限"
                />
              ) : (
                <SkeletonControl />
              )}
            </SettingRow>
          </KvListContent>
        </KvList>
      </SettingsSection>
    </div>
  );
}
