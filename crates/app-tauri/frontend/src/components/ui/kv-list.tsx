import * as React from "react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Check } from "@keyline-icons/react";

export interface KvListProps extends React.HTMLAttributes<HTMLDivElement> {}

export const KvList = React.forwardRef<HTMLDivElement, KvListProps>(
  ({ className, children, ...props }, ref) => {
    return (
      <div
        ref={ref}
        className={cn(
          "rounded-2xl border border-[color-mix(in_srgb,var(--husk-n200)_90%,transparent)] dark:border-white/10 bg-white dark:bg-active shadow-2xs overflow-hidden",
          className
        )}
        {...props}
      >
        {children}
      </div>
    );
  }
);
KvList.displayName = "KvList";

export interface KvListHeaderProps
  extends Omit<React.HTMLAttributes<HTMLDivElement>, "title"> {
  title: React.ReactNode;
  description?: React.ReactNode;
}

export const KvListHeader = React.forwardRef<HTMLDivElement, KvListHeaderProps>(
  ({ title, description, children, className, ...props }, ref) => {
    return (
      <div
        ref={ref}
        className={cn(
          "px-5 py-3.5 flex items-center justify-between bg-white dark:bg-active border-b border-neutral-100 dark:border-white/8",
          className
        )}
        {...props}
      >
        <div className="flex flex-col gap-0.5">
          <span className="text-sm font-semibold text-neutral-900 leading-none">
            {title}
          </span>
          {description && (
            <span className="text-xs text-neutral-500 mt-1">{description}</span>
          )}
        </div>
        {children && (
          <div className="flex items-center gap-2.5">{children}</div>
        )}
      </div>
    );
  }
);
KvListHeader.displayName = "KvListHeader";

export interface KvListContentProps
  extends React.HTMLAttributes<HTMLDivElement> {}

export const KvListContent = React.forwardRef<HTMLDivElement, KvListContentProps>(
  ({ className, children, ...props }, ref) => {
    return (
      <div
        ref={ref}
        className={cn("divide-y divide-neutral-100 text-sm", className)}
        {...props}
      >
        {children}
      </div>
    );
  }
);
KvListContent.displayName = "KvListContent";

export interface KvRowProps extends React.HTMLAttributes<HTMLDivElement> {
  label: React.ReactNode;
  description?: React.ReactNode;
  icon?: React.ReactNode;
}

export const KvRow = React.forwardRef<HTMLDivElement, KvRowProps>(
  ({ label, description, icon, children, className, ...props }, ref) => {
    return (
      <div
        ref={ref}
        className={cn(
          "px-5 py-3.5 min-h-[60px] flex items-center justify-between hover:bg-[color-mix(in_srgb,var(--husk-n50)_40%,transparent)] transition-colors text-sm",
          className
        )}
        {...props}
      >
        <div className="flex items-center gap-3 min-w-0 pr-4">
          {icon && <div className="text-neutral-500 shrink-0">{icon}</div>}
          <div className="flex flex-col">
            <span className="text-neutral-800 font-normal">{label}</span>
            {description && (
              <span className="text-xs text-neutral-500 mt-0.5">{description}</span>
            )}
          </div>
        </div>

        <div className="flex items-center gap-2 shrink-0">{children}</div>
      </div>
    );
  }
);
KvRow.displayName = "KvRow";

function isLightColor(hex: string): boolean {
  const cleanHex = hex.replace("#", "");
  if (cleanHex.length !== 6) return true;
  const r = parseInt(cleanHex.substring(0, 2), 16);
  const g = parseInt(cleanHex.substring(2, 4), 16);
  const b = parseInt(cleanHex.substring(4, 6), 16);
  const yiq = (r * 299 + g * 587 + b * 114) / 1000;
  return yiq >= 180;
}

const PRESET_COLORS = [
  "#339CFF",
  "#2563EB",
  "#0EA5E9",
  "#10B981",
  "#F59E0B",
  "#EF4444",
  "#8B5CF6",
  "#18181B",
  "#71717A",
  "#FFFFFF",
];

export interface ColorPillProps {
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
  className?: string;
}

export function ColorPill({
  value,
  onChange,
  disabled = false,
  className,
}: ColorPillProps) {
  const [open, setOpen] = React.useState(false);
  const light = isLightColor(value);

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={disabled}
          style={{ backgroundColor: light ? "#FFFFFF" : value }}
          className={cn(
            "h-8 px-4 rounded-xl flex items-center gap-2 text-xs font-semibold shadow-2xs transition-colors select-none border-[color-mix(in_srgb,var(--husk-n200)_90%,transparent)]",
            light
              ? "text-neutral-800 hover:bg-neutral-50"
              : "text-white hover:opacity-90 hover:text-white border-white/20",
            disabled && "opacity-50 cursor-not-allowed pointer-events-none",
            className
          )}
        >
          <span
            style={{ backgroundColor: value }}
            className={cn(
              "w-3 h-3 rounded-full shrink-0",
              light
                ? "border border-neutral-300"
                : "border border-[color-mix(in_srgb,var(--husk-white)_50%,transparent)] bg-[color-mix(in_srgb,var(--husk-white)_20%,transparent)]"
            )}
          />
          <span className="font-mono">{value}</span>
        </Button>
      </PopoverTrigger>

      <PopoverContent
        align="end"
        sideOffset={6}
        className="w-56 p-3 bg-white rounded-xl border border-neutral-200 shadow-popup select-none"
      >
        <div className="flex flex-col gap-2.5">
          <div className="flex items-center justify-between">
            <span className="text-xs font-semibold text-neutral-700">预设颜色</span>
            <span className="text-[11px] font-mono text-neutral-400">{value}</span>
          </div>

          <div className="grid grid-cols-5 gap-1.5">
            {PRESET_COLORS.map((c) => {
              const isSelected = value.toUpperCase() === c.toUpperCase();
              const isPresetLight = isLightColor(c);
              return (
                <Button
                  key={c}
                  type="button"
                  variant="outline"
                  size="icon"
                  onClick={() => onChange(c)}
                  style={{ backgroundColor: c }}
                  className={cn(
                    "w-8 h-8 rounded-lg flex items-center justify-center transition-colors cursor-pointer relative p-0",
                    isPresetLight ? "border border-neutral-200" : "border border-[color-mix(in_srgb,var(--husk-black)_10%,transparent)]",
                    isSelected && "ring-2 ring-blue-500 dark:ring-blue-400 ring-offset-1"
                  )}
                >
                  {isSelected && (
                    <Check
                      className={cn(
                        "w-3.5 h-3.5",
                        isPresetLight ? "text-neutral-900" : "text-white"
                      )}
                    />
                  )}
                </Button>
              );
            })}
          </div>

          <div className="h-px bg-neutral-100 my-0.5" />

          <div className="flex items-center gap-2">
            <div className="relative w-8 h-8 rounded-lg overflow-hidden border border-neutral-200 shrink-0 cursor-pointer">
              <input
                type="color"
                value={value}
                onChange={(e) => onChange(e.target.value.toUpperCase())}
                className="absolute -inset-2 w-12 h-12 cursor-pointer opacity-0"
              />
              <div
                style={{ backgroundColor: value }}
                className="w-full h-full rounded-lg"
              />
            </div>
            <Input
              type="text"
              value={value}
              onChange={(e) => onChange(e.target.value.toUpperCase())}
              placeholder="#FFFFFF"
              className="flex-1 rounded-xl"
            />
          </div>
        </div>
      </PopoverContent>
    </Popover>
  );
}

export const SettingsList = KvList;
export const SettingsListHeader = KvListHeader;
export const SettingsListContent = KvListContent;
export const SettingsRow = KvRow;
