// Shared loading placeholders for the settings pages. Same idiom as the chat
// skeleton: a pulsing block in `--husk-n200`, shaped like the real content so
// the swap lands in place instead of the page jumping.

import { cn } from "@/lib/utils";

/** A single pulsing block — the building material for every placeholder. */
export function Skeleton({
  className,
  style,
  ...rest
}: {
  className?: string;
  style?: React.CSSProperties;
} & React.AriaAttributes) {
  return (
    <div
      {...rest}
      style={style}
      className={cn(
        "rounded-md bg-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)]",
        className,
      )}
    />
  );
}

/** Rows shaped like `KvRow` — a label + description on the left, a control on
 * the right. Only the rows: the caller keeps its own frame, so a skeleton can
 * stand in for a list without replacing the section around it. */
export function SkeletonKvRows({ rows = 3 }: { rows?: number }) {
  return (
    <div aria-hidden="true" className="animate-pulse">
      {Array.from({ length: rows }, (_, i) => (
        <div
          key={i}
          className="px-5 py-3.5 min-h-[60px] flex items-center justify-between"
        >
          <div className="flex flex-col gap-1.5 min-w-0 pr-4">
            <Skeleton className="h-3.5 w-32" />
            <Skeleton className="h-2.5 w-48" />
          </div>
          <Skeleton className="h-7 w-24 shrink-0 rounded-lg" />
        </div>
      ))}
    </div>
  );
}

/** Rows inside their own `KvList` frame — for a whole list that has no
 * surrounding frame yet. */
export function SkeletonKvList({ rows = 3 }: { rows?: number }) {
  return (
    <div className="rounded-2xl border border-[color-mix(in_srgb,var(--husk-n200)_90%,transparent)] dark:border-white/10 bg-white dark:bg-active shadow-2xs overflow-hidden">
      <SkeletonKvRows rows={rows} />
    </div>
  );
}

/** Stands in for one settings control while its value loads — same footprint as
 * the real `SettingSelect` (h-8, min-w-44) or `Switch` (h-5 w-9), so the row
 * keeps its labels and the control fills in without any layout shift. */
export function SkeletonControl({ kind = "select" }: { kind?: "select" | "switch" }) {
  return (
    <Skeleton
      aria-hidden="true"
      className={cn(
        "animate-pulse shrink-0",
        kind === "select" ? "h-8 w-44 rounded-xl" : "h-5 w-9 rounded-full",
      )}
    />
  );
}
