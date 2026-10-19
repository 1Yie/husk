// Title-bar stat chip with a detail card. Hover previews the card, a click
// pins it open (outside click / Esc closes). Touch and pen skip the hover
// path, so a tap is just the pinned open.

import * as React from "react";
import * as PopoverPrimitive from "@radix-ui/react-popover";
import { cn } from "@/lib/utils";
import { OVERLAY_DIVIDER, SURFACE_OVERLAY } from "@/lib/theme";
import { POPPER_PENDING, usePopperPlacedRef } from "@/components/ui/use-popper-placed";

const OPEN_DELAY = 120;
const CLOSE_DELAY = 160;

export function StatPopover({
  label,
  trigger,
  children,
  className,
}: {
  /** Accessible name of the chip. */
  label: string;
  trigger: React.ReactNode;
  children: React.ReactNode;
  className?: string;
}) {
  const [open, setOpen] = React.useState(false);
  const [pinned, setPinned] = React.useState(false);
  const timer = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  // Only an Esc close hands focus back to the chip. Radix refocuses the
  // trigger on every close, and WebKit paints that programmatic focus as a
  // focus ring — a hover card closing as the pointer moves on left the chip
  // ringed.
  const escClosed = React.useRef(false);
  const { placed, setRef } = usePopperPlacedRef<HTMLDivElement>(null);

  const clear = () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
  };
  React.useEffect(() => clear, []);

  const hoverOpen = (e: React.PointerEvent) => {
    if (e.pointerType !== "mouse") return;
    clear();
    timer.current = setTimeout(() => setOpen(true), OPEN_DELAY);
  };
  const hoverClose = (e: React.PointerEvent) => {
    if (e.pointerType !== "mouse" || pinned) return;
    clear();
    timer.current = setTimeout(() => setOpen(false), CLOSE_DELAY);
  };
  const keepOpen = () => clear();

  return (
    <PopoverPrimitive.Root
      open={open}
      onOpenChange={(next) => {
        clear();
        setOpen(next);
        setPinned(next);
      }}
    >
      <PopoverPrimitive.Trigger asChild>
        <button
          type="button"
          data-tauri-drag-region="false"
          aria-label={label}
          data-pinned={pinned || undefined}
          onPointerEnter={hoverOpen}
          onPointerLeave={hoverClose}
          onClick={(e) => {
            // Own the toggle: a click on a hover-opened card pins it instead
            // of closing it.
            e.preventDefault();
            clear();
            if (open && !pinned) {
              setPinned(true);
            } else {
              setOpen(!open);
              setPinned(!open);
            }
          }}
          className={cn(
            "flex h-6 flex-none items-center gap-1.5 rounded-md px-1.5 text-[11px] leading-none tabular-nums text-neutral-600 transition-colors cursor-default",
            "outline-none focus-visible:ring-2 focus-visible:ring-[color-mix(in_srgb,var(--husk-accent)_45%,transparent)]",
            "hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] hover:text-neutral-800",
            "data-[state=open]:bg-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] data-[state=open]:text-neutral-800",
          )}
        >
          {trigger}
        </button>
      </PopoverPrimitive.Trigger>
      <PopoverPrimitive.Portal>
        <PopoverPrimitive.Content
          ref={setRef}
          side="bottom"
          align="end"
          sideOffset={6}
          collisionPadding={12}
          onOpenAutoFocus={(e) => {
            if (!pinned) e.preventDefault();
          }}
          onEscapeKeyDown={() => {
            escClosed.current = true;
          }}
          onCloseAutoFocus={(e) => {
            if (!escClosed.current) e.preventDefault();
            escClosed.current = false;
          }}
          onPointerEnter={keepOpen}
          onPointerLeave={hoverClose}
          className={cn(
            "z-50 w-72 p-0 text-[12px] text-neutral-700 shadow-popup outline-none select-none",
            "data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95 data-[side=bottom]:slide-in-from-top-1",
            SURFACE_OVERLAY,
            !placed && POPPER_PENDING,
            className,
          )}
        >
          {children}
        </PopoverPrimitive.Content>
      </PopoverPrimitive.Portal>
    </PopoverPrimitive.Root>
  );
}

/** Card header: icon + title on the left, the headline figure on the right. */
export function CardHeader({
  icon,
  title,
  value,
}: {
  icon?: React.ReactNode;
  title: React.ReactNode;
  value?: React.ReactNode;
}) {
  return (
    <div className="flex items-center gap-2 px-3.5 pt-3 pb-2.5">
      {icon && <span className="flex-none text-neutral-500">{icon}</span>}
      <span className="font-medium text-neutral-800">{title}</span>
      {value != null && (
        <span className="ml-auto tabular-nums text-neutral-800">{value}</span>
      )}
    </div>
  );
}

export function CardDivider() {
  return <div className={cn("mx-3.5 h-px", OVERLAY_DIVIDER)} />;
}

/** A label/value row; `swatch` prefixes a legend colour square. */
export function CardRow({
  label,
  value,
  swatch,
  muted,
}: {
  label: React.ReactNode;
  value: React.ReactNode;
  swatch?: string;
  muted?: boolean;
}) {
  return (
    <div className="flex items-center gap-2 px-3.5 py-1">
      {swatch && <span className={cn("h-2 w-2 flex-none rounded-[3px]", swatch)} />}
      <span className="text-neutral-500">{label}</span>
      <span className={cn("ml-auto tabular-nums", muted ? "text-neutral-500" : "text-neutral-800")}>
        {value}
      </span>
    </div>
  );
}

export function CardBody({ children }: { children: React.ReactNode }) {
  return <div className="py-2">{children}</div>;
}
