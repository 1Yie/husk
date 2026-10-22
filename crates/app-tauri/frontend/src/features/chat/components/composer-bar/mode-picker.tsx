// Agent-mode / permission-mode / thinking-level pickers — the three
// dropdowns on the left of the composer toolbar.

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuItem,
  DropdownMenuGroup,
} from "@/components/ui/dropdown-menu";
import {
  ChevronsUpDown,
  Check,
  Bot,
  FileText,
  Shield,
  PenLine,
  Zap,
  Ban,
  ShieldX,
  Hammer,
  Route,
  Flag,
  Plus,
  Folders,
} from "@keyline-icons/react";
import { useLinkedRoots } from "@/lib/linked-roots";
import { LinkedRootRow } from "@/components/linked-roots";
import { cn } from "@/lib/utils";
import { useState } from "react";

// Each mode carries its own icon so the trigger and the menu tell the modes
// apart at a glance, instead of one shield for every choice.
export const PERMISSION_MODES = [
  { value: "default", label: "默认", desc: "编辑与命令均需手动确认", icon: Shield },
  { value: "acceptEdits", label: "接受编辑", desc: "自动允许文件修改，终端命令仍需确认", icon: PenLine },
  { value: "auto", label: "自动", desc: "自动执行修改与常规命令，仅高危指令确认", icon: Zap },
  { value: "dontAsk", label: "不询问", desc: "仅允许只读操作，拒绝所有修改与命令", icon: Ban },
  { value: "bypassPermissions", label: "跳过权限", desc: "完全信任，自动跳过所有确认", icon: ShieldX },
] as const;

// Icons follow the mode's job: build makes things, plan charts a route,
// goal holds a flag to reach. `office` keeps the document glyph.
export const AGENT_MODES = [
  { value: "build", label: "构建", desc: "完整工具集 — 读写、执行、验证", icon: Hammer },
  { value: "plan", label: "计划", desc: "只读分析，产出实施方案，批准后执行", icon: Route },
  { value: "goal", label: "目标", desc: "自主推进直到目标达成或明确受阻", icon: Flag },
  { value: "office", label: "工作", desc: "Word / Excel / PPT — 创建与编辑 Office 文档", icon: FileText },
] as const;

/** The model picker's primary label: the `name` set in settings, falling back

/** The composer toolbar's left-side pickers: agent mode and permission mode.
 *  The thinking level is NOT here — it lives in the model picker on the right,
 *  so one menu chooses both what answers and how hard it thinks. */
export function ModePicker({
  agentMode,
  agentModeLabel,
  switchAgentMode,
  officeMode,
  mode,
  modeLabel,
  switchMode,
  workspaceRoot,
  sessionId,
}: {
  agentMode: string;
  agentModeLabel: string;
  switchAgentMode: (m: string) => Promise<void>;
  /** The active workspace IS the office pseudo-workspace — office is only
   *  pickable there; elsewhere it's a sidebar-level switch, not a
   *  per-session composer option. */
  officeMode?: boolean;
  mode: string;
  modeLabel: string;
  switchMode: (m: string) => Promise<void>;
  /** A linked-workspace grant is per session, so the menu needs the pair that
   *  names one. Absent → the entry stays hidden. */
  workspaceRoot?: string;
  sessionId?: number;
}) {
  const ModeIcon = PERMISSION_MODES.find((m) => m.value === mode)?.icon ?? Shield;
  const AgentModeIcon = AGENT_MODES.find((m) => m.value === agentMode)?.icon ?? Hammer;
  const links = useLinkedRoots(workspaceRoot, sessionId);
  // Where a new grant lands: this session by default (a grant should not
  // outlive the task unless the user says so), remembered on request.
  const [addScope, setAddScope] = useState<"session" | "workspace">("session");
  return (
    <>
        {/* Office mode is workspace-scoped — every session there is
            `office` by construction, so neither the agent-mode nor the
            permission picker applies. The thinking-level picker stays. */}
        {!officeMode && (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              aria-label="代理模式"
              className="flex items-center gap-1.5 min-w-0 px-2.5 py-1 text-[12px] font-medium text-neutral-600 bg-[color-mix(in_srgb,var(--husk-n100)_80%,transparent)] hover:bg-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] rounded-lg transition-colors border border-[color-mix(in_srgb,var(--husk-n200)_50%,transparent)] select-none cursor-pointer"
            >
              <AgentModeIcon className="h-3.5 w-3.5 text-neutral-500 shrink-0" />
              <span className="truncate max-w-[7em]">{agentModeLabel}</span>
              <ChevronsUpDown className="h-3.5 w-3.5 text-neutral-500 shrink-0" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" side="top" className="w-64">
            <DropdownMenuLabel className="text-xs text-neutral-500 font-normal">
              代理模式
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              {/* `office` never appears here — it's a workspace-level mode
                  switched from the sidebar tab, not a per-session pick. */}
              {AGENT_MODES.filter((m) => m.value !== "office").map((m) => {
                const active = m.value === agentMode;
                return (
                  <DropdownMenuItem
                    key={m.value}
                    onClick={() => void switchAgentMode(m.value)}
                    className="flex flex-col items-start py-2 px-2 cursor-pointer rounded-lg gap-0.5"
                  >
                    <div className="flex items-center justify-between w-full">
                      <span className="flex items-center gap-1.5 font-medium text-xs text-neutral-800">
                        <m.icon className="h-3.5 w-3.5 text-neutral-500 shrink-0" />
                        {m.label}
                      </span>
                      {active && <Check className="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400 shrink-0" />}
                    </div>
                    <span className="text-[11px] text-neutral-500 leading-tight">
                      {m.desc}
                    </span>
                  </DropdownMenuItem>
                );
              })}
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        )}

        {!officeMode && (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              aria-label="权限模式"
              className="flex items-center gap-1.5 min-w-0 px-2.5 py-1 text-[12px] font-medium text-neutral-600 bg-[color-mix(in_srgb,var(--husk-n100)_80%,transparent)] hover:bg-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] rounded-lg transition-colors border border-[color-mix(in_srgb,var(--husk-n200)_50%,transparent)] select-none cursor-pointer"
            >
              <ModeIcon className="h-3.5 w-3.5 text-neutral-500 shrink-0" />
              <span className="truncate max-w-[7em]">{modeLabel}</span>
              <ChevronsUpDown className="h-3.5 w-3.5 text-neutral-500 shrink-0" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" side="top" className="w-64">
            <DropdownMenuLabel className="text-xs text-neutral-500 font-normal">
              权限模式
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              {PERMISSION_MODES.map((m) => {
                const active = m.value === mode;
                return (
                  <DropdownMenuItem
                    key={m.value}
                    onClick={() => void switchMode(m.value)}
                    className="flex flex-col items-start py-2 px-2 cursor-pointer rounded-lg gap-0.5"
                  >
                    <div className="flex items-center justify-between w-full">
                      <span className="flex items-center gap-1.5 font-medium text-xs text-neutral-800">
                        <m.icon className="h-3.5 w-3.5 text-neutral-500 shrink-0" />
                        {m.label}
                      </span>
                      {active && <Check className="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400 shrink-0" />}
                    </div>
                    <span className="text-[11px] text-neutral-500 leading-tight">
                      {m.desc}
                    </span>
                  </DropdownMenuItem>
                );
              })}
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        )}

        {/* A grant is not a mode: it sits BESIDE the permission picker rather
            than inside its five choices. Same pill shape, so the toolbar still
            reads as one row of controls. */}
        {/* Re-read on open: a grant can land without this menu touching it —
            the model's `link_workspace` call, or another window's change. The
            hook only refetches on mount, so opening is the refresh point (and
            nothing is fetched while the menu stays closed). */}
        {!officeMode && links.ready && (
        <DropdownMenu
          onOpenChange={(open) => {
            if (open) void links.reload();
          }}
        >
          <DropdownMenuTrigger asChild>
            <button
              aria-label="关联工作区"
              className="flex items-center gap-1.5 min-w-0 px-2.5 py-1 text-[12px] font-medium text-neutral-600 bg-[color-mix(in_srgb,var(--husk-n100)_80%,transparent)] hover:bg-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] rounded-lg transition-colors border border-[color-mix(in_srgb,var(--husk-n200)_50%,transparent)] select-none cursor-pointer"
            >
              <Folders className="h-3.5 w-3.5 text-neutral-500 shrink-0" />
              <span className="truncate max-w-[7em]">关联工作区</span>
              <ChevronsUpDown className="h-3.5 w-3.5 text-neutral-500 shrink-0" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" side="top" className="w-[22rem] p-1">
            <DropdownMenuLabel className="text-xs text-neutral-500 font-normal">
              关联工作区
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            {links.all.length === 0 ? (
              <p className="px-2 py-1.5 text-[11px] text-neutral-500">
                还没有关联目录。跨仓库的项目可以在这里把另一个仓库加进来。
              </p>
            ) : (
              links.all.map((r) => (
                <LinkedRootRow
                  key={r.path}
                  root={r}
                  busy={links.busy}
                  onAccess={(root, access) => void links.setAccess(root.path, access)}
                  onRemove={(path) => void links.remove(path)}
                />
              ))
            )}
            <DropdownMenuSeparator />
            {/* One action, one line, left-to-right: the label hugs its text on
                the left, the scope control sits at the right edge. */}
            <div className="flex items-center justify-between gap-2 px-2 py-1.5">
              <button
                type="button"
                aria-disabled={links.busy}
                onClick={() => void links.pickAndAdd("readwrite", addScope === "workspace")}
                className="flex shrink-0 cursor-pointer items-center gap-1.5 rounded-lg px-1.5 py-1 text-xs text-neutral-700 transition-colors hover:bg-[color-mix(in_srgb,var(--husk-black)_5%,transparent)]"
              >
                <Plus className="h-3.5 w-3.5 shrink-0 text-neutral-500" />
                添加目录
              </button>
              <div className="flex shrink-0 items-center gap-0.5 rounded-full bg-[color-mix(in_srgb,var(--husk-black)_6%,transparent)] p-0.5">
                {(["session", "workspace"] as const).map((scope) => (
                  <button
                    key={scope}
                    type="button"
                    aria-pressed={addScope === scope}
                    onClick={() => setAddScope(scope)}
                    className={cn(
                      "cursor-pointer rounded-full px-2 py-0.5 text-[10px] transition-colors",
                      addScope === scope
                        ? "bg-white text-neutral-900 shadow-2xs"
                        : "text-neutral-500 hover:text-neutral-900",
                    )}
                  >
                    {scope === "session" ? "本会话" : "本工作区"}
                  </button>
                ))}
              </div>
            </div>
          </DropdownMenuContent>
        </DropdownMenu>
        )}

    </>
  );
}
