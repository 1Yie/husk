/** Shared settings list, filter, icon and empty-state components. */
import * as React from "react";
import type { ReactNode } from "react";
import { ChevronRight, Search, X } from "@keyline-icons/react";
import { Input } from "@/components/ui/input";
import { TruncatedText } from "@/components/ui/truncated-text";
import { cn } from "@/lib/utils";

export function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      {/* A compat flag's name overflows the column, so the label truncates and
          the tooltip carries the whole spelling — only while it is clipped. */}
      <TruncatedText
        text={label}
        side="top"
        className="block min-w-0 truncate text-xs text-neutral-600"
      />
      {children}
    </div>
  );
}

/* ---------------------------- toolbar ---------------------------- */

/** The row above a list: filters on the left, search + actions on the right.
 *  Wraps on narrow windows instead of squeezing the controls. */
export function PaneToolbar({ children }: { children: ReactNode }) {
  return <div className="flex flex-wrap items-center gap-2">{children}</div>;
}

/** Pill segmented control — the same shape as the fallback-chain controls. */
export function ScopeTabs<T extends string>({
  value,
  onChange,
  options,
}: {
  value: T;
  onChange: (v: T) => void;
  /** `count` undefined renders a dash (overview still loading). */
  options: { value: T; label: string; count?: number }[];
}) {
  return (
    <div className="flex shrink-0 items-center gap-0.5 rounded-full bg-neutral-100 p-0.5">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          onClick={() => onChange(o.value)}
          className={cn(
            "flex h-6 items-center gap-1.5 rounded-full px-2.5 text-xs transition-colors cursor-pointer",
            value === o.value
              ? "bg-white text-neutral-900 shadow-2xs"
              : "text-neutral-500 hover:text-neutral-900",
          )}
        >
          {o.label}
          <span
            className={cn(
              "tabular-nums text-[10px]",
              value === o.value ? "text-neutral-500" : "text-neutral-400",
            )}
          >
            {o.count ?? "–"}
          </span>
        </button>
      ))}
    </div>
  );
}

export function SearchBox({
  value,
  onChange,
  placeholder = "搜索名称或描述",
}: {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
}) {
  return (
    <div className="relative ml-auto w-full min-w-0 sm:w-56">
      <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-neutral-400" />
      <Input
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className="h-7 rounded-lg pl-8 pr-7 text-xs"
      />
      {value && (
        <button
          type="button"
          aria-label="清除搜索"
          onClick={() => onChange("")}
          className="absolute right-1.5 top-1/2 flex h-4 w-4 -translate-y-1/2 items-center justify-center rounded text-neutral-400 hover:text-neutral-700 cursor-pointer"
        >
          <X className="h-3 w-3" />
        </button>
      )}
    </div>
  );
}

/** Case-insensitive substring match over any of `fields`. */
export function matchQuery(query: string, ...fields: (string | undefined | null)[]) {
  const q = query.trim().toLowerCase();
  return !q || fields.some((f) => !!f && f.toLowerCase().includes(q));
}

/* ----------------------------- rows ------------------------------ */

/** Solid fills for a label that sits on the icon corner: the translucent
 *  `TAG_TONES` would let the icon show through. */
const BADGE_TONES = {
  neutral: "bg-neutral-300 text-neutral-700 dark:bg-neutral-300 dark:text-neutral-700",
  success: "bg-emerald-100 text-emerald-700 dark:bg-emerald-900 dark:text-emerald-300",
  warn: "bg-amber-100 text-amber-700 dark:bg-amber-900 dark:text-amber-300",
  danger: "bg-red-100 text-red-600 dark:bg-red-900 dark:text-red-300",
} as const;

const TAG_TONES = {
  neutral: "border-neutral-200 text-neutral-500",
  success:
    "border-transparent bg-emerald-500/15 text-emerald-600 dark:border-transparent dark:text-emerald-400",
  warn: "border-transparent bg-amber-500/15 text-amber-700 dark:border-transparent dark:text-amber-300",
  danger: "border-transparent bg-red-500/10 text-red-600 dark:border-transparent dark:text-red-400",
} as const;

/** Small rounded label beside an item title (scope, transport, state…). */
export function Tag({
  tone = "neutral",
  mono = false,
  icon,
  className,
  children,
}: {
  tone?: keyof typeof TAG_TONES;
  mono?: boolean;
  icon?: ReactNode;
  className?: string;
  children: ReactNode;
}) {
  return (
    <span
      className={cn(
        "inline-flex h-4 shrink-0 items-center gap-1 rounded-full border px-1.5 text-[10px] font-medium leading-none",
        mono && "font-mono",
        TAG_TONES[tone],
        className,
      )}
    >
      {icon}
      {children}
    </span>
  );
}

const DOT_TONES = {
  success: "bg-emerald-500",
  danger: "bg-red-500",
  warn: "bg-amber-500",
  idle: "bg-neutral-300",
} as const;

/** The tinted square that leads every item row; `status` pins a live dot to
 *  its corner (connection state and the like), `badge` a small value label
 *  (a percentage, a count) in the same corner. Use one or the other. */
export function ItemIcon({
  children,
  status,
  badge,
  className,
}: {
  children: ReactNode;
  status?: keyof typeof DOT_TONES;
  badge?: { label: ReactNode; tone?: keyof typeof BADGE_TONES };
  className?: string;
}) {
  return (
    <div className={cn("relative mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-neutral-100 text-neutral-500", className)}>
      {children}
      {status && (
        <span
          className={cn(
            "absolute -bottom-0.5 -right-0.5 h-2.5 w-2.5 rounded-full ring-2 ring-white dark:ring-[var(--husk-active)]",
            DOT_TONES[status],
          )}
        />
      )}
      {badge && (
        <span
          className={cn(
            "absolute -bottom-1 -right-1 flex h-3.5 items-center rounded-full px-1 text-[9px] font-semibold leading-none tabular-nums ring-2 ring-white dark:ring-[var(--husk-active)]",
            BADGE_TONES[badge.tone ?? "neutral"],
          )}
        >
          {/* Digits sit high in this font's line box; nudge them to the pill's optical centre. */}
          <span className="relative top-px">{badge.label}</span>
        </span>
      )}
    </div>
  );
}

export interface ItemRowProps extends Omit<React.HTMLAttributes<HTMLDivElement>, "title"> {
  icon: ReactNode;
  title: ReactNode;
  /** Monospace title — ids and slugs. */
  monoTitle?: boolean;
  /** Let a sentence-length title wrap to two lines instead of truncating. */
  wrapTitle?: boolean;
  /** Tags right after the title. */
  tags?: ReactNode;
  description?: ReactNode;
  /** Placeholder shown when `description` is empty; omit to hide the line. */
  emptyDescription?: string;
  /** Third line — a path, a command, a URL. Full text on hover. */
  meta?: string;
  /** Right-hand controls (buttons, switches). Clicks don't reach the row. */
  actions?: ReactNode;
  /** Set on expandable rows — draws the disclosure chevron. */
  expanded?: boolean;
}

/** One entry of a list card: icon · title + tags · description · meta, with
 *  actions on the right. Forwards its ref so it can be a `CollapsibleTrigger`. */
export const ItemRow = React.forwardRef<HTMLDivElement, ItemRowProps>(
  (
    {
      icon,
      title,
      monoTitle = false,
      wrapTitle = false,
      tags,
      description,
      emptyDescription,
      meta,
      actions,
      expanded,
      className,
      ...props
    },
    ref,
  ) => {
    const expandable = expanded !== undefined;
    return (
      <div
        ref={ref}
        className={cn(
          "group flex items-start gap-3 px-5 py-3.5 transition-colors hover:bg-[color-mix(in_srgb,var(--husk-n50)_40%,transparent)]",
          expandable && "cursor-pointer",
          className,
        )}
        {...props}
      >
        {icon}

        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <div className={cn("flex min-w-0 gap-1.5", wrapTitle ? "items-start" : "items-center")}>
            <span
              className={cn(
                "text-neutral-800",
                wrapTitle ? "line-clamp-2 leading-snug" : "truncate",
                monoTitle ? "font-mono text-[13px]" : "text-sm",
              )}
            >
              {title}
            </span>
            {tags}
          </div>
          {description ? (
            <span className="line-clamp-2 text-xs leading-relaxed text-neutral-500">{description}</span>
          ) : emptyDescription ? (
            <span className="text-xs italic text-neutral-400">{emptyDescription}</span>
          ) : null}
          {meta && (
            <span
              title={meta}
              className="w-fit max-w-full truncate font-mono text-[11px] text-neutral-400"
            >
              {meta}
            </span>
          )}
        </div>

        {actions && (
          <div
            className="flex shrink-0 items-center gap-0.5 self-center"
            onClick={(e) => e.stopPropagation()}
          >
            {actions}
          </div>
        )}
        {expandable && (
          <ChevronRight
            className={cn(
              "h-3.5 w-3.5 shrink-0 self-center text-neutral-400 transition-transform",
              expanded && "rotate-90",
            )}
          />
        )}
      </div>
    );
  },
);
ItemRow.displayName = "ItemRow";

/** Preference row with room for a select. Controls wrap below the label when
 *  the settings content is narrow, regardless of the overall window width. */
export function SettingRow({
  icon,
  title,
  description,
  children,
  wideControl = false,
}: {
  icon: ReactNode;
  title: string;
  description?: string;
  children: ReactNode;
  wideControl?: boolean;
}) {
  const titleId = React.useId();
  return (
    <div role="group" aria-labelledby={titleId} className="flex flex-wrap items-center gap-x-4 gap-y-3 px-5 py-4">
      <div className="flex min-w-0 flex-[1_1_220px] items-center gap-3">
        <ItemIcon className="mt-0">{icon}</ItemIcon>
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <span id={titleId} className="text-sm text-neutral-800">{title}</span>
          {description && <span className="text-xs leading-relaxed text-neutral-500">{description}</span>}
        </div>
      </div>
      <div className={cn(
        "ml-auto flex min-w-0 max-w-full items-center justify-end",
        wideControl ? "flex-[0_1_200px] [&>button]:w-full [&>button]:min-w-0 [&>button]:max-w-full" : "shrink-0",
      )}>
        {children}
      </div>
    </div>
  );
}

/** Indented detail block under an expanded `ItemRow` — aligned with the text
 *  column, not the icon. */
export function ItemDetail({ children }: { children: ReactNode }) {
  return <div className="flex flex-col gap-2.5 pb-3.5 pl-16 pr-5">{children}</div>;
}

/** A labelled chip group inside `ItemDetail` (工具 / 钩子 / 命令). */
export function ChipGroup({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <span className="text-[11px] font-semibold uppercase tracking-wide text-neutral-400">{label}</span>
      <div className="flex flex-wrap gap-1.5">{children}</div>
    </div>
  );
}

/** Chip inside a `ChipGroup`. */
export function Chip({ icon, children }: { icon?: ReactNode; children: ReactNode }) {
  return (
    <span className="inline-flex h-5 items-center gap-1 rounded-md border border-neutral-200 bg-white px-1.5 font-mono text-[10.5px] text-neutral-600 dark:border-[var(--husk-border)] dark:bg-transparent">
      {icon}
      {children}
    </span>
  );
}

/* --------------------------- empty state -------------------------- */

/** Centered placeholder for a list with nothing to show. */
export function EmptyState({
  icon,
  title,
  description,
  action,
}: {
  icon: ReactNode;
  title: ReactNode;
  description?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center gap-2 px-5 py-10 text-center">
      <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-neutral-100 text-neutral-500">
        {icon}
      </div>
      <span className="text-sm text-neutral-800">{title}</span>
      {description && <span className="text-xs text-neutral-500">{description}</span>}
      {action && <div className="mt-1">{action}</div>}
    </div>
  );
}
