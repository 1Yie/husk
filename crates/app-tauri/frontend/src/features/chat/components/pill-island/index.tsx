import type { ReactNode } from "react";
import { TooltipSimple } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

/** One capsule on the row above the composer. The row decides whether its
 *  text shows; the caller only says what the capsule is. */
export interface IslandPill {
  key: string;
  /** Accessible name — also what tests find the button by. */
  label: string;
  tooltip: ReactNode;
  icon: ReactNode;
  text: ReactNode;
  onClick?: () => void;
  pressed?: boolean;
  /** Extra `data-*` hooks the caller styles or tests against. */
  data?: Record<`data-${string}`, string | undefined>;
}

/** The Dynamic-Island rule: with two or more capsules up, the last one is an
 *  icon until the pointer or the keyboard asks for its text; every capsule
 *  ahead of it speaks in full. */
export function pillCollapsed(index: number, count: number): boolean {
  return count >= 2 && index === count - 1;
}

/** One class pair so expanded and collapsed text can never animate differently. */
const PILL_TEXT = "flex min-w-0 items-center gap-1.5 overflow-hidden transition-all duration-200 ease-out";
const PILL_TEXT_OPEN = "ml-1.5 max-w-[180px] opacity-100";
const PILL_TEXT_COLLAPSED =
  "ml-0 max-w-0 opacity-0 " +
  "group-hover:ml-1.5 group-hover:max-w-[180px] group-hover:opacity-100 " +
  "group-focus-visible:ml-1.5 group-focus-visible:max-w-[180px] group-focus-visible:opacity-100";

const PILL_BUTTON = cn(
  "group flex h-8 items-center gap-0 rounded-full border px-2 text-[12px] transition-all duration-200 ease-out active:scale-95 cursor-pointer",
  "border-[color-mix(in_srgb,var(--husk-n200)_90%,transparent)] shadow-[0_2px_8px_rgba(0,0,0,0.08)] dark:shadow-[0_2px_8px_rgba(0,0,0,0.35)]",
  // One look whether the panel behind it is open or closed — an open panel is
  // not a selected tab. Hover is the only state change.
  // Foreground tokens already follow the theme; a second neutral inversion
  // makes small dark-mode labels nearly the same colour as the capsule.
  "bg-white text-text-secondary hover:bg-neutral-50 hover:text-text-primary dark:bg-[var(--husk-active)] dark:hover:bg-[var(--husk-hover)]",
);

/** The capsules above the composer as one row. Spacing is the row's `gap`, so
 *  it is the same between any two capsules whichever of them are up. The row
 *  sits inside the stream's `select-text` root, so it opts out explicitly: a
 *  drag across the stream must not paint a highlight into the capsules. */
export function PillIsland({ pills }: { pills: IslandPill[] }) {
  if (pills.length === 0) return null;
  return (
    <div data-pill-island="" className="flex select-none items-center gap-2">
      {pills.map((pill, i) => {
        const collapsed = pillCollapsed(i, pills.length);
        return (
          <TooltipSimple key={pill.key} content={pill.tooltip} side="top">
            <button
              type="button"
              onClick={pill.onClick}
              aria-label={pill.label}
              aria-pressed={pill.pressed}
              data-compact={collapsed ? "true" : undefined}
              {...pill.data}
              className={PILL_BUTTON}
            >
              {pill.icon}
              <span className={cn(PILL_TEXT, collapsed ? PILL_TEXT_COLLAPSED : PILL_TEXT_OPEN)}>
                {pill.text}
              </span>
            </button>
          </TooltipSimple>
        );
      })}
    </div>
  );
}
