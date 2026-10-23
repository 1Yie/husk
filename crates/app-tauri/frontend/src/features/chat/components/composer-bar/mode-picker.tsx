// Agent-mode / permission-mode pickers and the 关联工作区 grant menu — the left
// side of the composer toolbar. A grant is not a mode, so it sits BESIDE the
// permission picker rather than inside its choices, wearing the same pill chrome
// so the row still reads as one group of controls.
//
// Every control follows the toolbar's density tier: labels when the row is wide,
// icons when it is not, and one collapsed menu when even the icons would crowd
// the row. Nothing shrinks below its icon — a squeezed pill is worse than none.

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuItem,
  DropdownMenuGroup,
} from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import {
  ChevronsUpDown,
  Check,
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
  SlidersHorizontal,
} from "@keyline-icons/react";
import { useLinkedRoots } from "@/lib/linked-roots";
import { LinkedRootRow } from "@/components/linked-roots";
import { cn } from "@/lib/utils";
import type { ToolbarDensity } from "@/features/chat/components/composer-bar/toolbar-density";
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

type ModeSpec = (typeof AGENT_MODES)[number] | (typeof PERMISSION_MODES)[number];

/** Pill chrome shared by every left-hand control. `shrink-0` at every tier: a
 *  pill allowed to shrink collapses into a bordered sliver with no label. */
const PILL =
  "flex shrink-0 items-center gap-1.5 px-2.5 py-1 text-[12px] font-medium text-neutral-600 bg-[color-mix(in_srgb,var(--husk-n100)_80%,transparent)] hover:bg-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] rounded-lg transition-colors border border-[color-mix(in_srgb,var(--husk-n200)_50%,transparent)] select-none cursor-pointer";

/** Icon-only square: the same chrome with one glyph and neither label nor
 *  chevron to crowd it. */
const ICON_PILL = `${PILL} h-7 justify-center px-1.5`;

/** A picker's trigger in its two tiers — labelled pill, or icon-only square
 *  whose tooltip and aria-label carry the text the label gave up. Returns the
 *  DropdownMenuTrigger itself so the caller keeps owning the menu content. */
function PickerTrigger({
  labelled,
  ariaLabel,
  iconLabel,
  label,
  icon,
}: {
  labelled: boolean;
  /** The short name the trigger carries while its label is on screen. */
  ariaLabel: string;
  /** The full sentence the icon-only trigger carries instead — also its tooltip. */
  iconLabel: string;
  label: string;
  icon: React.ReactNode;
}) {
  const trigger = (
    <DropdownMenuTrigger asChild>
      <button aria-label={labelled ? ariaLabel : iconLabel} className={labelled ? PILL : ICON_PILL}>
        {icon}
        {labelled && (
          <>
            <span className="truncate max-w-[7em]">{label}</span>
            <ChevronsUpDown className="h-3.5 w-3.5 text-neutral-500 shrink-0" />
          </>
        )}
      </button>
    </DropdownMenuTrigger>
  );
  if (labelled) return trigger;
  return (
    <Tooltip>
      <TooltipTrigger asChild>{trigger}</TooltipTrigger>
      <TooltipContent side="top">{iconLabel}</TooltipContent>
    </Tooltip>
  );
}

/** One mode choice: the mode's icon and label on the first line, what it means
 *  underneath. Shared by every tier's menu, so the collapsed menu at the
 *  tightest density reads exactly like the full ones. */
function ModeItem({
  mode,
  active,
  onSelect,
}: {
  mode: ModeSpec;
  active: boolean;
  onSelect: (value: string) => void;
}) {
  const Icon = mode.icon;
  return (
    <DropdownMenuItem
      onClick={() => onSelect(mode.value)}
      className="flex flex-col items-start py-2 px-2 cursor-pointer rounded-lg gap-0.5"
    >
      <div className="flex items-center justify-between w-full">
        <span className="flex items-center gap-1.5 font-medium text-xs text-neutral-800">
          <Icon className="h-3.5 w-3.5 text-neutral-500 shrink-0" />
          {mode.label}
        </span>
        {active && <Check className="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400 shrink-0" />}
      </div>
      <span className="text-[11px] text-neutral-500 leading-tight">{mode.desc}</span>
    </DropdownMenuItem>
  );
}

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
  density = "full",
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
  /** The toolbar's own width tier (see toolbar-density.ts). */
  density?: ToolbarDensity;
}) {
  const ModeIcon = PERMISSION_MODES.find((m) => m.value === mode)?.icon ?? Shield;
  const AgentModeIcon = AGENT_MODES.find((m) => m.value === agentMode)?.icon ?? Hammer;
  const links = useLinkedRoots(workspaceRoot, sessionId);
  // Where a new grant lands: this session by default (a grant should not
  // outlive the task unless the user says so), remembered on request.
  const [addScope, setAddScope] = useState<"session" | "workspace">("session");
  // `office` never appears in the agent-mode menu — it's a workspace-level mode
  // switched from the sidebar tab, not a per-session pick.
  const agentModes = AGENT_MODES.filter((m) => m.value !== "office");
  const labelled = density === "full";

  return (
    <>
      {/* The tightest row cannot carry three borders: both modes fold into one
          menu, as labelled sections so either choice stays one click away.
          关联工作区 keeps its own button below — its menu is a list of grants,
          not a mode choice, and folding it in would need a second level. */}
      {!officeMode && density === "tight" && (
        <DropdownMenu>
          <Tooltip>
            <TooltipTrigger asChild>
              <DropdownMenuTrigger asChild>
                <button aria-label="模式与权限" className={ICON_PILL}>
                  <SlidersHorizontal className="h-3.5 w-3.5 text-neutral-500" />
                </button>
              </DropdownMenuTrigger>
            </TooltipTrigger>
            <TooltipContent side="top">模式与权限</TooltipContent>
          </Tooltip>
          <DropdownMenuContent align="start" side="top" className="w-64">
            <DropdownMenuLabel className="text-xs text-neutral-500 font-normal">
              代理模式
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              {agentModes.map((m) => (
                <ModeItem
                  key={m.value}
                  mode={m}
                  active={m.value === agentMode}
                  onSelect={(value) => void switchAgentMode(value)}
                />
              ))}
            </DropdownMenuGroup>
            <DropdownMenuSeparator />
            <DropdownMenuLabel className="text-xs text-neutral-500 font-normal">
              权限模式
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              {PERMISSION_MODES.map((m) => (
                <ModeItem
                  key={m.value}
                  mode={m}
                  active={m.value === mode}
                  onSelect={(value) => void switchMode(value)}
                />
              ))}
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      )}

      {/* Office mode is workspace-scoped — every session there is `office` by
          construction, so neither the agent-mode nor the permission picker
          applies. The thinking-level picker stays. */}
      {!officeMode && density !== "tight" && (
        <DropdownMenu>
          <PickerTrigger
            labelled={labelled}
            ariaLabel="代理模式"
            iconLabel={`代理模式：${agentModeLabel}`}
            label={agentModeLabel}
            icon={<AgentModeIcon className="h-3.5 w-3.5 text-neutral-500 shrink-0" />}
          />
          <DropdownMenuContent align="start" side="top" className="w-64">
            <DropdownMenuLabel className="text-xs text-neutral-500 font-normal">
              代理模式
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              {agentModes.map((m) => (
                <ModeItem
                  key={m.value}
                  mode={m}
                  active={m.value === agentMode}
                  onSelect={(value) => void switchAgentMode(value)}
                />
              ))}
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      )}

      {!officeMode && density !== "tight" && (
        <DropdownMenu>
          <PickerTrigger
            labelled={labelled}
            ariaLabel="权限模式"
            iconLabel={`权限模式：${modeLabel}`}
            label={modeLabel}
            icon={<ModeIcon className="h-3.5 w-3.5 text-neutral-500 shrink-0" />}
          />
          <DropdownMenuContent align="start" side="top" className="w-64">
            <DropdownMenuLabel className="text-xs text-neutral-500 font-normal">
              权限模式
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              {PERMISSION_MODES.map((m) => (
                <ModeItem
                  key={m.value}
                  mode={m}
                  active={m.value === mode}
                  onSelect={(value) => void switchMode(value)}
                />
              ))}
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      )}

      {/* Re-read on open: a grant can land without this menu touching it — the
          model's `link_workspace` call, or another window's change. The hook
          only refetches on mount, so opening is the refresh point (and nothing
          is fetched while the menu stays closed). */}
      {!officeMode && links.ready && (
        <DropdownMenu
          onOpenChange={(open) => {
            if (open) void links.reload();
          }}
        >
          <PickerTrigger
            labelled={labelled}
            ariaLabel="关联工作区"
            iconLabel="关联工作区"
            label="关联工作区"
            icon={<Folders className="h-3.5 w-3.5 text-neutral-500 shrink-0" />}
          />
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
