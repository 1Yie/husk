import * as React from "react";
import { ChevronsUpDown } from "@keyline-icons/react";
import { cn } from "@/lib/utils";
import {
  KvList,
  KvListHeader,
  KvListContent,
  KvRow,
  ColorPill,
} from "@/components/ui/kv-list";
import { Input } from "@/components/ui/input";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
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
            "flex h-8 min-w-44 items-center justify-between gap-2 whitespace-nowrap rounded-xl border border-neutral-200 bg-white px-3 text-xs text-neutral-800 shadow-sm transition-colors focus:outline-none focus:ring-1 focus:ring-neutral-900 data-[state=open]:ring-1 data-[state=open]:ring-neutral-900",
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
/* Field schema                                                        */
/* ------------------------------------------------------------------ */

interface FieldBase {
  /** unique within the section — used as the React key */
  key: string;
  label: React.ReactNode;
  description?: React.ReactNode;
  icon?: React.ReactNode;
}

export type SettingField =
  | (FieldBase & {
      type: "select";
      value: string;
      onChange: (v: string) => void;
      options: SettingSelectOption[];
      placeholder?: string;
      prefix?: React.ReactNode;
    })
  | (FieldBase & {
      type: "color";
      value: string;
      onChange: (v: string) => void;
    })
  | (FieldBase & {
      type: "input";
      value: string;
      onChange: (v: string) => void;
      placeholder?: string;
      /** tailwind width class for the input, defaults to w-64 */
      width?: string;
    })
  | (FieldBase & {
      type: "slider";
      value: number;
      onChange: (v: number) => void;
      min?: number;
      max?: number;
      step?: number;
    })
  | (FieldBase & {
      type: "switch";
      value: boolean;
      onChange: (v: boolean) => void;
    })
  | (FieldBase & {
      /** escape hatch for one-off controls — prefer adding a real type */
      type: "custom";
      render: () => React.ReactNode;
    });

function FieldControl({ field }: { field: SettingField }) {
  switch (field.type) {
    case "select":
      return (
        <SettingSelect
          value={field.value}
          onChange={field.onChange}
          options={field.options}
          placeholder={field.placeholder}
          prefix={field.prefix}
        />
      );
    case "color":
      return <ColorPill value={field.value} onChange={field.onChange} />;
    case "input":
      return (
        <Input
          value={field.value}
          onChange={(e) => field.onChange(e.target.value)}
          placeholder={field.placeholder}
          className={cn("rounded-xl", field.width ?? "w-64")}
        />
      );
    case "slider":
      return (
        <div className="flex items-center gap-4">
          <Slider
            value={[field.value]}
            onValueChange={([v]) => field.onChange(v)}
            min={field.min ?? 0}
            max={field.max ?? 100}
            step={field.step ?? 1}
            className="w-48"
          />
          <span className="w-6 select-none text-right font-mono text-xs font-medium text-neutral-800">
            {field.value}
          </span>
        </div>
      );
    case "switch":
      return <Switch checked={field.value} onCheckedChange={field.onChange} />;
    case "custom":
      return <>{field.render()}</>;
  }
}

/* ------------------------------------------------------------------ */
/* Section schema + renderer                                           */
/* ------------------------------------------------------------------ */

export interface SettingsCardOption {
  value: string;
  label: React.ReactNode;
  description?: React.ReactNode;
  badge?: React.ReactNode;
  icon?: React.ReactNode;
}

export type SettingsSection =
  | {
      kind: "list";
      key: string;
      title: React.ReactNode;
      description?: React.ReactNode;
      /** free-form actions rendered in the list header (buttons, SettingSelect…) */
      actions?: React.ReactNode;
      fields: SettingField[];
    }
  | {
      kind: "cards";
      key: string;
      title: React.ReactNode;
      description?: React.ReactNode;
      value: string;
      onChange: (v: string) => void;
      options: SettingsCardOption[];
    };

function CardsSection({
  section,
}: {
  section: Extract<SettingsSection, { kind: "cards" }>;
}) {
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-col gap-0.5">
        <span className="text-sm font-medium text-neutral-800">
          {section.title}
        </span>
        {section.description && (
          <span className="text-xs text-neutral-500">{section.description}</span>
        )}
      </div>
      <div className="grid gap-2.5">
        {section.options.map((opt) => {
          const isSelected = section.value === opt.value;
          return (
            <Card
              key={opt.value}
              onClick={() => section.onChange(opt.value)}
              className={cn(
                "p-3.5 cursor-pointer transition-all duration-150 shadow-none",
                isSelected
                  ? "border-neutral-900 bg-neutral-50/80 ring-1 ring-neutral-900"
                  : "border-neutral-200/90 bg-white hover:border-neutral-300 hover:bg-neutral-50/40"
              )}
            >
              <div className="flex items-start gap-3.5">
                {opt.icon && <div className="mt-0.5">{opt.icon}</div>}
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-semibold text-neutral-900">
                      {opt.label}
                    </span>
                    {opt.badge}
                  </div>
                  {opt.description && (
                    <p className="mt-1 text-xs leading-relaxed text-neutral-500">
                      {opt.description}
                    </p>
                  )}
                </div>
                <div className="shrink-0 pt-0.5">
                  <div
                    className={cn(
                      "flex h-4 w-4 items-center justify-center rounded-full border transition-colors",
                      isSelected
                        ? "border-neutral-900 bg-neutral-900"
                        : "border-neutral-300 bg-white"
                    )}
                  >
                    {isSelected && (
                      <div className="h-1.5 w-1.5 rounded-full bg-white" />
                    )}
                  </div>
                </div>
              </div>
            </Card>
          );
        })}
      </div>
    </div>
  );
}

function ListSection({
  section,
}: {
  section: Extract<SettingsSection, { kind: "list" }>;
}) {
  return (
    <KvList>
      <KvListHeader title={section.title} description={section.description}>
        {section.actions}
      </KvListHeader>
      <KvListContent>
        {section.fields.map((field) => (
          <KvRow
            key={field.key}
            label={field.label}
            description={field.description}
            icon={field.icon}
          >
            <FieldControl field={field} />
          </KvRow>
        ))}
      </KvListContent>
    </KvList>
  );
}

/** Declarative settings renderer — declare sections/fields as data, controls
 * and layout render automatically. */
export function SettingsRenderer({
  sections,
}: {
  sections: SettingsSection[];
}) {
  return (
    <>
      {sections.map((s) =>
        s.kind === "cards" ? (
          <CardsSection key={s.key} section={s} />
        ) : (
          <ListSection key={s.key} section={s} />
        )
      )}
    </>
  );
}
