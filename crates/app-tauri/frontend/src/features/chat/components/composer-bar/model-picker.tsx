// ModelPicker — what answers and how hard it thinks, in one popover with two
// levels: the root states the current pair, and each row drills into its own
// list with a back arrow at the top-left. The trigger keeps the same summary
// (model + tier) it always had.

import { useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { TruncatedText } from "@/components/ui/truncated-text";
import { ReasoningEffort } from "@/components/ui/reasoning-effort";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { THINKING_LEVELS } from "@/lib/thinking";
import { groupOptions } from "@/lib/group-options";
import { cn } from "@/lib/utils";
import type { ToolbarDensity } from "@/features/chat/components/composer-bar/toolbar-density";
import {
  Bot,
  Brain,
  Check,
  ChevronLeft,
  ChevronRight,
  ChevronsUpDown,
} from "@keyline-icons/react";
import type * as agent from "@/lib/agent-ipc/index";

/** The picker's primary label: the `name` set in settings, falling back to the
 *  wire id when the config declares none. */
export function modelLabel(m: agent.ModelItem): string {
  return m.name?.trim() || m.model;
}

/** The picker's small second line — the raw model id, what actually goes on
 *  the wire and what `/model` expects. Empty when the id IS the label, so a
 *  nameless model never prints its own name twice; the provider is the row's
 *  section heading, not a sub-label. */
function modelSubLabel(m: agent.ModelItem): string {
  return modelLabel(m) !== m.model ? m.model : "";
}

/** Full name of a thinking tier — `THINKING_LEVELS` is the one list. */
function tierLabel(value: string): string {
  return THINKING_LEVELS.find((level) => level.value === value)?.label ?? value;
}

/** A row in the root level: label on the left, the current value on the right. */
function SummaryRow({
  icon,
  label,
  value,
  disabled,
  onOpen,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
  disabled?: boolean;
  onOpen: () => void;
}) {
  return (
    <DropdownMenuItem
      disabled={disabled}
      // The row only navigates: the popover must stay open for the level it
      // opens (Radix closes a menu on select by default).
      onSelect={(e) => {
        e.preventDefault();
        onOpen();
      }}
      className="flex min-w-0 cursor-pointer items-center gap-2 px-2 py-2"
    >
      <span className="flex-none text-neutral-500">{icon}</span>
      <span className="flex-none text-xs text-neutral-600">{label}</span>
      <span className="min-w-0 flex-1" />
      <TruncatedText
        side="left"
        text={value}
        className="min-w-0 truncate text-xs font-medium text-neutral-900"
      />
      <ChevronRight className="size-3.5 flex-none text-neutral-400" />
    </DropdownMenuItem>
  );
}

/** The header every drilled-in level carries: back arrow, title, and whatever
 *  the level wants on the right (a wire mapping, a count). */
function LevelHeader({ title, onBack, trailing }: {
  title: string;
  onBack: () => void;
  trailing?: React.ReactNode;
}) {
  return (
    <div className="flex items-center gap-1 px-2 py-1">
      <button
        type="button"
        aria-label="返回"
        onClick={(e) => {
          e.stopPropagation();
          onBack();
        }}
        className="flex size-5 flex-none items-center justify-center rounded-md text-neutral-500 transition-colors hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] hover:text-neutral-800"
      >
        <ChevronLeft className="size-4" />
      </button>
      <span className="text-xs font-normal leading-5 text-neutral-500">{title}</span>
      {trailing && <span className="ml-auto pr-1">{trailing}</span>}
    </div>
  );
}

type View = "root" | "models" | "thinking";

/** Cross-fades between the picker's levels and animates the box height, so a
 *  drill-in grows out of the summary instead of hard-swapping from one size to
 *  the next. The measured child is a sibling of the animated box, not inside
 *  it, so its natural height is never confused with the height being tweened. */
function LevelSwitch({ view, children }: { view: View; children: React.ReactNode }) {
  const measureRef = useRef<HTMLDivElement>(null);
  const [height, setHeight] = useState<number | undefined>(undefined);
  // The first measurement lands with the popover's own opening animation: no
  // tween from zero.
  const settled = useRef(false);

  useLayoutEffect(() => {
    const el = measureRef.current;
    if (!el) return;
    const apply = () => {
      // Clamped to the popover's own ceiling; the lists scroll inside it.
      setHeight(Math.min(el.offsetHeight, 320));
      settled.current = true;
    };
    apply();
    const ro = new ResizeObserver(apply);
    ro.observe(el);
    return () => ro.disconnect();
  }, [view]);

  return (
    <div
      className={cn("overflow-hidden", settled.current && "transition-[height] duration-200 ease-out")}
      style={{ height }}
    >
      <div ref={measureRef} key={view}>
        {children}
      </div>
    </div>
  );
}

export function ModelPicker({
  activeModel,
  activeProvider,
  models,
  onSelectModel,
  hasReasoning,
  activeThinkingLevel,
  thinkingMap,
  effectiveThinkingLevels,
  onSelectThinkingLevel,
  open: openProp,
  onOpenChange,
  density = "full",
}: {
  activeModel: string;
  activeProvider: string;
  models: agent.ModelItem[];
  onSelectModel: (provider: string, model: string) => Promise<void>;
  hasReasoning: boolean;
  activeThinkingLevel: string;
  thinkingMap?: Record<string, string | null>;
  effectiveThinkingLevels: readonly { value: string; token: string }[];
  onSelectThinkingLevel: (level: string) => Promise<void>;
  /** Controlled open state — `/model` opens the picker from the panel, which
   *  the trigger alone cannot do. Uncontrolled when absent. */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** The toolbar's width tier — caps the name, then drops the tier's word. */
  density?: ToolbarDensity;
}) {
  const [view, setView] = useState<View>("root");
  const [selfOpen, setSelfOpen] = useState(false);
  const open = openProp ?? selfOpen;
  const setOpen = (next: boolean) => {
    setSelfOpen(next);
    // A fresh open always starts at the summary — a half-drilled popover from
    // a minute ago is not what the trigger's label promised.
    if (!next) setView("root");
    onOpenChange?.(next);
  };

  // The active model's config entry — supplies the display name. `undefined`
  // when the model isn't in the configured list (a hand-encoded `/model`
  // target, or a config edited outside the app): the raw id is then the only
  // thing we can show.
  const activeItem = models.find(
    (model) => model.model === activeModel && model.provider === activeProvider,
  );
  const modelSections = useMemo(
    () =>
      groupOptions(
        models,
        (m) => m.provider || undefined,
        (a, b) => modelLabel(a).localeCompare(modelLabel(b)),
      ),
    [models],
  );
  /** The wire value the active tier rewrites to, when the model remaps it
   *  (e.g. picking Low sends `max`) — shown so the control never lies. */
  const thinkingOverride =
    thinkingMap?.[activeThinkingLevel] &&
    thinkingMap[activeThinkingLevel] !== activeThinkingLevel
      ? thinkingMap[activeThinkingLevel]
      : null;
  const tiers = effectiveThinkingLevels.map((level) => ({
    value: level.value,
    token: level.token,
    label: tierLabel(level.value),
  }));
  const thinkingAvailable = hasReasoning && tiers.length > 0;

  /** How much of the model name the trigger may print at this tier — the name
   *  is the only thing on the toolbar row allowed to give way. */
  const nameCap =
    density === "full" ? "max-w-[140px]" : density === "compact" ? "max-w-[96px]" : "max-w-[72px]";
  const activeTierToken =
    tiers.find((l) => l.value === activeThinkingLevel)?.token ?? activeThinkingLevel;

  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger asChild>
        <button
          aria-label={`当前模型: ${activeItem ? modelLabel(activeItem) : activeModel || "未选择"} (${activeProvider}/${activeModel})`}
          className={cn(
            "flex items-center gap-1.5 min-w-0 px-2.5 py-1 text-[12px] font-medium text-neutral-600 hover:bg-neutral-100 rounded-lg transition-colors select-none cursor-pointer",
            // Below full the chip keeps its size and lets the name truncate.
            density !== "full" && "shrink-0",
          )}
        >
          <span
            className={cn(
              "truncate min-w-0",
              nameCap,
              // No display name → the label IS the id; keep the old mono
              // look instead of re-styling an identifier as prose.
              (!activeItem || modelLabel(activeItem) === activeModel) && "font-mono text-[11.5px]",
            )}
          >
            {activeItem ? modelLabel(activeItem) : activeModel || "选择模型"}
          </span>
          {/* The thinking level rides with the model — one control picks both
              what answers and how hard it thinks. Below full its word is the
              first thing to go: the icon and a tooltip say it in 12px. */}
          {thinkingAvailable && density !== "tight" && (
            <span className="flex items-center gap-1 shrink-0 pl-1.5 border-l border-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)]">
              {density === "full" ? (
                <>
                  <Brain
                    className={cn(
                      "h-3 w-3 shrink-0",
                      activeThinkingLevel === "off"
                        ? "text-neutral-400"
                        : "text-purple-600 dark:text-purple-400",
                    )}
                  />
                  <span className="truncate max-w-[5em] text-[11px] text-neutral-500">
                    {activeTierToken}
                  </span>
                </>
              ) : (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span className="flex items-center">
                      <Brain
                        className={cn(
                          "h-3 w-3 shrink-0",
                          activeThinkingLevel === "off"
                            ? "text-neutral-400"
                            : "text-purple-600 dark:text-purple-400",
                        )}
                      />
                    </span>
                  </TooltipTrigger>
                  <TooltipContent side="top">思考：{activeTierToken}</TooltipContent>
                </Tooltip>
              )}
            </span>
          )}
          <ChevronsUpDown className="h-3.5 w-3.5 text-neutral-500 shrink-0" />
        </button>
      </DropdownMenuTrigger>

      <DropdownMenuContent
        align="end"
        side="top"
        className="w-80 max-h-[min(20rem,var(--radix-dropdown-menu-content-available-height))] overflow-hidden"
      >
        <LevelSwitch view={view}>
        {view === "root" && (
          <>
            <DropdownMenuLabel className="text-xs text-neutral-500 font-normal">
              模型与思考
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              <SummaryRow
                icon={<Bot className="size-3.5" />}
                label="模型"
                value={activeItem ? modelLabel(activeItem) : activeModel || "未选择"}
                onOpen={() => setView("models")}
              />
              <SummaryRow
                icon={
                  <Brain
                    className={cn(
                      "size-3.5",
                      activeThinkingLevel === "off" && "text-neutral-400",
                    )}
                  />
                }
                label="思考"
                value={thinkingAvailable ? tierLabel(activeThinkingLevel) : "该模型不支持思考等级"}
                disabled={!thinkingAvailable}
                onOpen={() => setView("thinking")}
              />
            </DropdownMenuGroup>
          </>
        )}

        {view === "models" && (
          <>
            <LevelHeader title="模型" onBack={() => setView("root")} />
            <DropdownMenuSeparator />
            {models.length > 0 ? (
              <DropdownMenuGroup className="max-h-[17rem] overflow-y-auto overflow-x-hidden">
                {/* One section per provider, alphabetical — the provider map
                    upstream carries no order of its own. */}
                {modelSections.map((section, i) => (
                  <div key={section.group ?? `solo-${i}`}>
                    {i > 0 && <DropdownMenuSeparator />}
                    {section.group && (
                      <DropdownMenuLabel className="truncate text-[11px] font-medium text-neutral-700">
                        {section.group}
                      </DropdownMenuLabel>
                    )}
                    {section.items.map((item) => {
                      const active =
                        item.model === activeModel && item.provider === activeProvider;
                      return (
                        <DropdownMenuItem
                          key={`${item.provider}/${item.model}`}
                          onClick={() => {
                            void onSelectModel(item.provider, item.model);
                            setView("root");
                          }}
                          className="flex min-w-0 items-center justify-between overflow-hidden text-xs py-2 cursor-pointer"
                        >
                          <div className="flex min-w-0 flex-1 flex-col pr-2">
                            <TruncatedText
                              text={modelLabel(item)}
                              className="truncate font-medium text-neutral-900"
                            />
                            {modelSubLabel(item) && (
                              <TruncatedText
                                text={modelSubLabel(item)}
                                className="truncate font-mono text-[10px] text-neutral-500"
                              />
                            )}
                          </div>
                          {active && (
                            <Check className="size-3.5 shrink-0 text-emerald-600 dark:text-emerald-400" />
                          )}
                        </DropdownMenuItem>
                      );
                    })}
                  </div>
                ))}
              </DropdownMenuGroup>
            ) : (
              <div className="px-3 py-3 text-xs text-neutral-500 text-center">未检测到模型</div>
            )}
          </>
        )}

        {view === "thinking" && (
          <>
            <LevelHeader
              title="思考"
              onBack={() => setView("root")}
              trailing={
                thinkingOverride ? (
                  <span className="font-mono text-[10px] text-neutral-400">→ {thinkingOverride}</span>
                ) : undefined
              }
            />
            <DropdownMenuSeparator />
            <div className="px-3 pb-2.5 pt-1.5">
              {/* The stepped control: stops follow the active model's own
                  tiers, and one gesture reports the tier it landed on. */}
              <ReasoningEffort
                stops={tiers.map((tier) => tier.label)}
                value={Math.max(0, tiers.findIndex((tier) => tier.value === activeThinkingLevel))}
                label={activeItem ? modelLabel(activeItem) : activeModel}
                ariaLabel="思考等级"
                onChange={(index) => {
                  const tier = tiers[index];
                  if (tier && tier.value !== activeThinkingLevel) {
                    void onSelectThinkingLevel(tier.value);
                  }
                }}
              />
            </div>
          </>
        )}
        </LevelSwitch>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
