import * as React from "react";
import { Check, ChevronsUpDown } from "@keyline-icons/react";
import { cn } from "@/lib/utils";
import { ItemIcon } from "./item-list";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

export * from "./item-list";

/* ------------------------------------------------------------------ */
/* Controls                                                            */
/* ------------------------------------------------------------------ */

export interface SettingSelectOption {
  value: string;
  label: React.ReactNode;
  /** muted trailing text after the label, e.g. "— 不使用额外推理" */
  description?: React.ReactNode;
}

interface SettingSelectProps {
  value: string;
  onChange: (value: string) => void;
  options: SettingSelectOption[];
  placeholder?: string;
  /** node rendered before the label inside the trigger (e.g. the "Aa" badge) */
  prefix?: React.ReactNode;
  triggerClassName?: string;
  contentClassName?: string;
  ariaLabel?: string;
  disabled?: boolean;
}

/** The single dropdown control for settings. Backed by DropdownMenu whose
 * shared wrapper already forces modal={false} — Radix `Select` locks body
 * scroll via RemoveScroll and must not come back. */
export function SettingSelect({
  value,
  onChange,
  options,
  placeholder = "请选择",
  prefix,
  triggerClassName,
  contentClassName,
  ariaLabel,
  disabled,
}: SettingSelectProps) {
  const current = options.find((o) => o.value === value);
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label={ariaLabel}
          disabled={disabled}
          className={cn(
            "flex h-8 min-w-44 max-w-full items-center justify-between gap-2 whitespace-nowrap rounded-xl border border-neutral-200 bg-white px-3 text-xs text-neutral-800 shadow-sm transition-[color,box-shadow] focus:outline-none focus:border-neutral-400 focus:ring-[3px] focus:ring-[color-mix(in_srgb,var(--husk-n950)_12%,transparent)] data-[state=open]:border-neutral-400 data-[state=open]:ring-[3px] data-[state=open]:ring-[color-mix(in_srgb,var(--husk-n950)_12%,transparent)] disabled:pointer-events-none disabled:opacity-50 dark:border-hairline dark:bg-hover",
            triggerClassName
          )}
        >
          <span className="flex min-w-0 items-center gap-2">
            {prefix}
            <span className="truncate">{current?.label ?? placeholder}</span>
          </span>
          <ChevronsUpDown className="h-3.5 w-3.5 shrink-0 opacity-50" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className={cn(
          "min-w-[var(--radix-dropdown-menu-trigger-width)] max-w-[calc(100vw-2rem)]",
          contentClassName
        )}
      >
        <DropdownMenuRadioGroup value={value} onValueChange={onChange}>
          {options.map((o) => (
            <DropdownMenuRadioItem
              key={o.value}
              value={o.value}
              className="text-xs py-2 cursor-pointer"
            >
              <span className="font-medium text-neutral-900">{o.label}</span>
              {o.description && (
                <span className="text-neutral-500 text-xs ml-2">
                  — {o.description}
                </span>
              )}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/* ------------------------------------------------------------------ */
/* Section components                                                  */
/* ------------------------------------------------------------------ */

export interface SettingsCardOption {
  value: string;
  label: React.ReactNode;
  description?: React.ReactNode;
  badge?: React.ReactNode;
  icon?: React.ReactNode;
  preview?: React.ReactNode;
}

/** Section shell — the title/description/actions header every settings
 * block shares, with arbitrary content (usually a `KvList`) underneath.
 * The single section primitive every settings pane composes — put a
 * `KvList`, `SettingsCards`, or any custom block inside. */
export function SettingsSection({
  title,
  description,
  actions,
  children,
}: {
  title?: React.ReactNode;
  description?: React.ReactNode;
  /** free-form actions rendered in the header's right column (buttons…) */
  actions?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="flex min-w-0 flex-col gap-3">
      {(title || description || actions) && (
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="flex min-w-0 flex-col gap-1">
            {title && (
              <span className="text-sm font-semibold text-neutral-900">
                {title}
              </span>
            )}
            {description && (
              <span className="text-xs leading-relaxed text-neutral-500">{description}</span>
            )}
          </div>
          {actions && (
            <div className="ml-auto flex shrink-0 items-center gap-2">{actions}</div>
          )}
        </div>
      )}
      {children}
    </div>
  );
}

/** Grid of selectable cards — usually sits inside a
 * `SettingsSection`. */
export function SettingsCards({
  value,
  onChange,
  options,
  className,
}: {
  value: string;
  onChange: (v: string) => void;
  options: SettingsCardOption[];
  className?: string;
}) {
  return (
    <div className={cn("grid grid-cols-[repeat(auto-fit,minmax(min(100%,240px),1fr))] gap-3", className)}>
      {options.map((opt) => {
        const isSelected = value === opt.value;
        return (
          <button
            key={opt.value}
            type="button"
            aria-pressed={isSelected}
            onClick={() => onChange(opt.value)}
            className={cn(
              "flex min-w-0 cursor-pointer flex-col gap-3 rounded-2xl border bg-card p-4 text-left shadow-2xs transition-colors select-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--husk-white)]",
              isSelected
                ? "border-accent bg-[color-mix(in_srgb,var(--husk-accent)_4%,var(--husk-card))] ring-1 ring-[color-mix(in_srgb,var(--husk-accent)_25%,transparent)]"
                : "border-hairline hover:border-neutral-300"
            )}
          >
            {opt.preview}
            <div className="flex w-full items-center gap-3">
              {opt.icon && <ItemIcon className="mt-0">{opt.icon}</ItemIcon>}
              <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
                <span className="text-sm font-medium text-neutral-900">
                  {opt.label}
                </span>
                {opt.badge}
              </div>
              <span
                aria-hidden="true"
                className={cn(
                  "flex h-4 w-4 shrink-0 items-center justify-center rounded-full border transition-colors",
                  isSelected ? "border-accent bg-accent" : "border-neutral-300"
                )}
              >
                {isSelected && <Check className="h-2.5 w-2.5 text-zinc-50" />}
              </span>
            </div>
            {opt.description && (
              <p className="text-xs leading-relaxed text-neutral-500">
                {opt.description}
              </p>
            )}
          </button>
        );
      })}
    </div>
  );
}
