import { useLayoutEffect, useRef, useState } from "react";
import { Bot, FileDiff, FolderOpen, House, MoreHorizontal, Package, SquareTerminal } from "lucide-react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";
import type { DockTab } from "./state";

const META = {
  tools: { Icon: House, label: "工具", unit: "项" },
  files: { Icon: FolderOpen, label: "文件", unit: "项" },
  changes: { Icon: FileDiff, label: "改动", unit: "个文件" },
  artifacts: { Icon: Package, label: "产物", unit: "个文件" },
  jobs: { Icon: SquareTerminal, label: "后台", unit: "个任务" },
  subagents: { Icon: Bot, label: "子代理", unit: "个" },
};
const TAB = "flex h-9 flex-none items-center gap-1 whitespace-nowrap px-2 text-[12px]";
const CONTROL = "flex h-6 w-6 flex-none items-center justify-center rounded-md text-neutral-500 transition-colors hover:bg-[var(--husk-hover)] hover:text-neutral-800 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-[var(--husk-focus)] motion-reduce:transition-none";
const labelFor = (tab: DockTab, count: number) =>
  count > 0 ? `${META[tab].label}（${count} ${META[tab].unit}）` : META[tab].label;

/** Reserve room for the overflow button, then favor the current tab. */
export function visibleDockTabs(tabs: DockTab[], widths: number[], available: number, active: DockTab): DockTab[] {
  const total = widths.reduce((sum, width) => sum + width, 0) + Math.max(0, tabs.length - 1) * 4;
  if (total <= available) return tabs;
  let remaining = available - 28;
  const visible = new Set<DockTab>();
  const order = tabs.includes(active) ? [active, ...tabs.filter((tab) => tab !== active)] : tabs;
  for (const tab of order) {
    const width = widths[tabs.indexOf(tab)];
    if (width <= remaining) {
      visible.add(tab);
      remaining -= width + 4;
    }
  }
  return tabs.filter((tab) => visible.has(tab));
}

function Caption({ tab, count }: { tab: DockTab; count: number }) {
  const { Icon, label } = META[tab];
  return <>
    <Icon className="h-3.5 w-3.5 flex-none" aria-hidden="true" />
    <span>{label}</span>
    {count > 0 && <span className="font-mono text-[10.5px] tabular-nums text-neutral-400">{count}</span>}
  </>;
}

export function DockTabs({ tabs, counts, active, onSelect }: {
  tabs: DockTab[];
  counts: Record<DockTab, number>;
  active: DockTab;
  onSelect: (tab: DockTab) => void;
}) {
  const lane = useRef<HTMLDivElement>(null);
  const measure = useRef<HTMLDivElement>(null);
  const more = useRef<HTMLButtonElement>(null);
  const selectedFromMenu = useRef<DockTab | null>(null);
  const dismissedByKeyboard = useRef(false);
  const [visible, setVisible] = useState<DockTab[] | null>(null);
  const signature = tabs.map((tab) => `${tab}:${counts[tab]}`).join("|");

  useLayoutEffect(() => {
    const host = lane.current;
    const ruler = measure.current;
    if (!host || !ruler) return;
    const update = () => {
      const available = host.getBoundingClientRect().width;
      if (!available) {
        setVisible(null);
        return;
      }
      const widths = Array.from(ruler.children, (child) => child.getBoundingClientRect().width);
      const next = visibleDockTabs(tabs, widths, available, active);
      setVisible((previous) => previous?.join("|") === next.join("|") ? previous : next);
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(host);
    observer.observe(ruler);
    return () => observer.disconnect();
  }, [signature, active, tabs]);

  const shown = tabs.filter((tab) => !visible || visible.includes(tab));
  const hidden = tabs.filter((tab) => visible && !visible.includes(tab));
  return (
    <div className="relative flex h-9 w-full flex-none items-center gap-1 border-b border-hairline px-2">
      {/* Intrinsic widths remain measurable even when their real tabs are in the menu. */}
      <div aria-hidden="true" className="pointer-events-none invisible absolute inset-0 overflow-hidden">
        <div ref={measure} data-dock-tabs-measure className="flex w-max">
          {tabs.map((tab) => <span key={tab} className={cn(TAB, active === tab && "font-medium")}>
            <Caption tab={tab} count={counts[tab]} />
          </span>)}
        </div>
      </div>
      <div ref={lane} data-dock-tabs-lane className="flex h-full min-w-0 flex-1 items-center gap-1">
        <div role="tablist" aria-label="侧栏" className="flex h-full min-w-0 items-center gap-1">
          {shown.map((tab) => (
            <button key={tab} type="button" role="tab" data-dock-tab={tab}
              aria-selected={active === tab} aria-label={labelFor(tab, counts[tab])}
              onClick={() => onSelect(tab)}
              className={cn(TAB,
                "transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-[var(--husk-focus)] motion-reduce:transition-none",
                active === tab ? "font-medium text-neutral-800" : "text-neutral-500 hover:text-neutral-800",
              )}>
              <Caption tab={tab} count={counts[tab]} />
            </button>
          ))}
        </div>
        {hidden.length > 0 && (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button ref={more} type="button" aria-label="更多工作区标签" title="更多" className={CONTROL}>
                <MoreHorizontal className="h-4 w-4" aria-hidden="true" />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="min-w-36"
              onEscapeKeyDown={() => { dismissedByKeyboard.current = true; }}
              onCloseAutoFocus={(event) => {
                const tab = selectedFromMenu.current;
                selectedFromMenu.current = null;
                if (tab || dismissedByKeyboard.current) {
                  event.preventDefault();
                  const button = tab && lane.current?.querySelector<HTMLButtonElement>(`[data-dock-tab="${tab}"]`);
                  (button || more.current)?.focus();
                }
                dismissedByKeyboard.current = false;
              }}>
              {hidden.map((tab) => {
                const { Icon, label } = META[tab];
                return <DropdownMenuItem key={tab} aria-label={labelFor(tab, counts[tab])}
                  className="gap-2 text-xs"
                  onSelect={() => { selectedFromMenu.current = tab; onSelect(tab); }}>
                  <Icon className="h-3.5 w-3.5" aria-hidden="true" />
                  <span>{label}</span>
                  {counts[tab] > 0 && <span className="ml-auto pl-4 font-mono text-[11px] tabular-nums text-neutral-500">{counts[tab]}</span>}
                </DropdownMenuItem>;
              })}
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </div>
      <button type="button" onClick={() => onSelect("tools")} aria-label="工作区工具"
        aria-pressed={active === "tools"} title="工作区工具"
        className={cn(CONTROL, active === "tools" && "text-neutral-800")}>
        <House className="h-3.5 w-3.5" aria-hidden="true" />
      </button>
    </div>
  );
}
