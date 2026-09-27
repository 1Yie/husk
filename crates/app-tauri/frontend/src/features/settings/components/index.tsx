import * as React from "react";
import { ChevronsUpDown } from "@keyline-icons/react";
import { cn } from "@/lib/utils";
import { Card } from "@/components/ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

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
}: SettingSelectProps) {
  const current = options.find((o) => o.value === value);
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className={cn(
            "flex h-8 min-w-44 items-center justify-between gap-2 whitespace-nowrap rounded-xl border border-neutral-200 bg-white px-3 text-xs text-neutral-800 shadow-sm transition-[color,box-shadow] focus:outline-none focus:border-neutral-400 focus:ring-[3px] focus:ring-[color-mix(in_srgb,var(--husk-n950)_12%,transparent)] data-[state=open]:border-neutral-400 data-[state=open]:ring-[3px] data-[state=open]:ring-[color-mix(in_srgb,var(--husk-n950)_12%,transparent)]",
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
          "min-w-[var(--radix-dropdown-menu-trigger-width)]",
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
    <div className="flex flex-col gap-3">
      {(title || description || actions) && (
        <div className="flex items-center justify-between">
          <div className="flex flex-col gap-0.5">
            {title && (
              <span className="text-sm font-semibold text-neutral-900">
                {title}
              </span>
            )}
            {description && (
              <span className="text-xs text-neutral-500">{description}</span>
            )}
          </div>
          {actions && (
            <div className="flex items-center gap-2">{actions}</div>
          )}
        </div>
      )}
      {children}
    </div>
  );
}

/** Grid of selectable cards (radio semantics) — usually sits inside a
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
    <div className={cn("grid grid-cols-1 sm:grid-cols-2 gap-3", className)}>
      {options.map((opt) => {
        const isSelected = value === opt.value;
        return (
          <Card
            key={opt.value}
            onClick={() => onChange(opt.value)}
            className={cn(
              "p-4 cursor-pointer transition-all duration-150 shadow-none rounded-2xl flex flex-col justify-between select-none",
              isSelected
                ? "border-accent ring-1 ring-accent/35 bg-[color-mix(in_srgb,var(--husk-accent)_4%,transparent)] shadow-xs"
                : "border-[color-mix(in_srgb,var(--husk-n200)_90%,transparent)] bg-white hover:border-neutral-300 hover:bg-[color-mix(in_srgb,var(--husk-n50)_40%,transparent)]"
            )}
          >
            <div>
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-2 min-w-0">
                  {opt.icon && <div className="shrink-0">{opt.icon}</div>}
                  <span className="text-sm font-semibold text-neutral-900 truncate">
                    {opt.label}
                  </span>
                  {opt.badge}
                </div>
                <div className="shrink-0">
                  <div
                    className={cn(
                      "flex h-4 w-4 items-center justify-center rounded-full border transition-colors",
                      isSelected
                        ? "border-accent bg-accent"
                        : "border-neutral-300 bg-white"
                    )}
                  >
                    {isSelected && (
                      <div className="h-1.5 w-1.5 rounded-full bg-white" />
                    )}
                  </div>
                </div>
              </div>
              {opt.description && (
                <p className="mt-2 text-xs leading-relaxed text-neutral-500">
                  {opt.description}
                </p>
              )}
            </div>
          </Card>
        );
      })}
    </div>
  );
}
