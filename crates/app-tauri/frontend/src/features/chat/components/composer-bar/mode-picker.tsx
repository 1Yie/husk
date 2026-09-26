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
  ShieldCheck,
  Check,
  Brain,
  Bot,
} from "@keyline-icons/react";
import { cn } from "@/lib/utils";

export const PERMISSION_MODES = [
  { value: "default", label: "默认", desc: "编辑与命令均需手动确认" },
  { value: "acceptEdits", label: "接受编辑", desc: "自动允许文件修改，终端命令仍需确认" },
  { value: "auto", label: "自动", desc: "自动执行修改与常规命令，仅高危指令确认" },
  { value: "dontAsk", label: "不询问", desc: "仅允许只读操作，拒绝所有修改与命令" },
  { value: "bypassPermissions", label: "跳过权限", desc: "完全信任，自动跳过所有确认" },
] as const;

export const AGENT_MODES = [
  { value: "build", label: "构建", desc: "完整工具集 — 读写、执行、验证" },
  { value: "plan", label: "计划", desc: "只读分析，产出实施方案，批准后执行" },
  { value: "goal", label: "目标", desc: "自主推进直到目标达成或明确受阻" },
] as const;

export const THINKING_LEVELS = [
  { value: "off", label: "关闭思考", desc: "不使用推理计算" },
  { value: "minimal", label: "最小推理", desc: "最轻量推理深度" },
  { value: "low", label: "轻度思考", desc: "快速简短的分析" },
  { value: "medium", label: "中等思考", desc: "标准平衡思考" },
  { value: "high", label: "深度思考", desc: "深度推演与设计" },
  { value: "xhigh", label: "超高思考", desc: "超深层推演" },
  { value: "max", label: "最大推理", desc: "最大算力深度推演" },
] as const;

/** Suffix shown after a level's label — the raw wire value, or
 *  `value→mapped` when the active model's thinking_level_map rewrites it
 *  (e.g. low→max means picking low sends `max` to the model). */
export function thinkingSuffix(value: string, map?: Record<string, string | null>): string {
  const mapped = map?.[value];
  return typeof mapped === "string" && mapped && mapped !== value
    ? `${value}→${mapped}`
    : value;
}

/** The model picker's primary label: the `name` set in settings, falling back

/** The composer toolbar's left-side pickers: agent mode, permission mode,
 * and (when the model reasons) thinking level. */
export function ModePicker({
  agentMode,
  agentModeLabel,
  switchAgentMode,
  mode,
  modeLabel,
  switchMode,
  hasReasoning,
  activeThinkingLevel,
  currentThinkingLabel,
  thinkingMap,
  effectiveThinkingLevels,
  onSelectThinkingLevel,
}: {
  agentMode: string;
  agentModeLabel: string;
  switchAgentMode: (m: string) => Promise<void>;
  mode: string;
  modeLabel: string;
  switchMode: (m: string) => Promise<void>;
  hasReasoning: boolean;
  activeThinkingLevel: string;
  currentThinkingLabel: string;
  thinkingMap?: Record<string, string | null>;
  effectiveThinkingLevels: readonly { value: string; label: string; desc: string }[];
  onSelectThinkingLevel: (level: string) => Promise<void>;
}) {
  return (
    <>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              aria-label="代理模式"
              className="flex items-center gap-1.5 min-w-0 px-2.5 py-1 text-[12px] font-medium text-neutral-600 bg-[color-mix(in_srgb,var(--husk-n100)_80%,transparent)] hover:bg-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] rounded-lg transition-colors border border-[color-mix(in_srgb,var(--husk-n200)_50%,transparent)] select-none cursor-pointer"
            >
              <Bot className="h-3.5 w-3.5 text-neutral-500 shrink-0" />
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
              {AGENT_MODES.map((m) => {
                const active = m.value === agentMode;
                return (
                  <DropdownMenuItem
                    key={m.value}
                    onClick={() => void switchAgentMode(m.value)}
                    className="flex flex-col items-start py-2 px-2 cursor-pointer rounded-lg gap-0.5"
                  >
                    <div className="flex items-center justify-between w-full">
                      <span className="font-medium text-xs text-neutral-800">
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

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              aria-label="权限模式"
              className="flex items-center gap-1.5 min-w-0 px-2.5 py-1 text-[12px] font-medium text-neutral-600 bg-[color-mix(in_srgb,var(--husk-n100)_80%,transparent)] hover:bg-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] rounded-lg transition-colors border border-[color-mix(in_srgb,var(--husk-n200)_50%,transparent)] select-none cursor-pointer"
            >
              <ShieldCheck className="h-3.5 w-3.5 text-neutral-500 shrink-0" />
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
                      <span className="font-medium text-xs text-neutral-800">
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

        {hasReasoning && effectiveThinkingLevels.length > 0 && (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button
              aria-label={`思考推理强度: ${currentThinkingLabel}`}
              className="flex items-center gap-1.5 min-w-0 px-2.5 py-1 text-[12px] font-medium text-neutral-600 bg-[color-mix(in_srgb,var(--husk-n100)_80%,transparent)] hover:bg-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] rounded-lg transition-colors border border-[color-mix(in_srgb,var(--husk-n200)_50%,transparent)] select-none cursor-pointer"
              >
                <Brain
                  className={cn(
                    "h-3.5 w-3.5 shrink-0",
                    activeThinkingLevel === "off"
                      ? "text-neutral-500"
                      : "text-purple-600 dark:text-purple-400",
                  )}
                />
                <span className="truncate max-w-[140px]">
                  {currentThinkingLabel}
                </span>
                <ChevronsUpDown className="h-3.5 w-3.5 text-neutral-500 shrink-0" />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" side="top" className="w-48">
              <DropdownMenuLabel className="text-xs text-neutral-500 font-normal">
                思考推理强度
              </DropdownMenuLabel>
              <DropdownMenuSeparator />
              <DropdownMenuGroup>
                {effectiveThinkingLevels.map((lvl) => {
                  const active = lvl.value === activeThinkingLevel;
                  return (
                    <DropdownMenuItem
                      key={lvl.value}
                      onClick={() => void onSelectThinkingLevel(lvl.value)}
                      className="flex items-center justify-between text-xs py-2 cursor-pointer"
                    >
                      <div className="flex flex-col gap-0.5 min-w-0 pr-2">
                        <span className="font-medium text-neutral-800">
                          {lvl.label}（{thinkingSuffix(lvl.value, thinkingMap)}）
                        </span>
                        <span className="text-[10.5px] text-neutral-500 truncate">
                          {lvl.desc}
                        </span>
                      </div>
                      {active && (
                        <Check className="h-3.5 w-3.5 text-purple-600 dark:text-purple-400 shrink-0" />
                      )}
                    </DropdownMenuItem>
                  );
                })}
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        )}
    </>
  );
}
