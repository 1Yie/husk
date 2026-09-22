import * as React from "react";
import * as TooltipPrimitive from "@radix-ui/react-tooltip";

import { cn } from "@/lib/utils";
import { usePopperPlacedRef } from "./use-popper-placed";

const TooltipProvider = TooltipPrimitive.Provider;
const Tooltip = TooltipPrimitive.Root;
const TooltipTrigger = TooltipPrimitive.Trigger;

const TooltipContent = React.forwardRef<
  React.ElementRef<typeof TooltipPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TooltipPrimitive.Content>
>(({ className, sideOffset = 6, collisionPadding = 12, ...props }, ref) => {
  const { placed, setRef } = usePopperPlacedRef(ref);
  return (
  <TooltipPrimitive.Portal>
    <TooltipPrimitive.Content
      ref={setRef}
      sideOffset={sideOffset}
      collisionPadding={collisionPadding}
      className={cn(
        "z-50 overflow-hidden rounded-md bg-neutral-900 px-2.5 py-1 text-xs text-neutral-50 shadow-md",
        "animate-in fade-in-0 zoom-in-95",
        !placed && "invisible",
        "data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95",
        "data-[side=bottom]:slide-in-from-top-1 data-[side=top]:slide-in-from-bottom-1",
        "data-[side=left]:slide-in-from-right-1 data-[side=right]:slide-in-from-left-1",
        " border border-[color-mix(in_srgb,var(--husk-n700)_50%,transparent)] select-none",
        className
      )}
      {...props}
    />
  </TooltipPrimitive.Portal>
  );
});
TooltipContent.displayName = TooltipPrimitive.Content.displayName;

/**
 * Convenient single-element tooltip wrapper with collision avoidance.
 */
export function TooltipSimple({
  content,
  children,
  side = "top",
  sideOffset = 6,
  className,
}: {
  content: React.ReactNode;
  children: React.ReactNode;
  side?: "top" | "bottom" | "left" | "right";
  sideOffset?: number;
  className?: string;
}) {
  if (!content) return <>{children}</>;
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent side={side} sideOffset={sideOffset} className={className}>
        {content}
      </TooltipContent>
    </Tooltip>
  );
}

/**
 * Global fallback tooltip engine:
 * 1. Automatically intercepts any unhandled native `title` attributes across the DOM,
 *    stripping them so the browser never shows its ugly unstyled OS tooltip.
 * 2. Accurately clamps position inside window boundaries so it NEVER clips or gets cut off.
 * 3. Pure opacity fade without diagonal translate displacement.
 */
export function GlobalTooltip() {
  const [target, setTarget] = React.useState<{
    text: string;
    rect: DOMRect;
  } | null>(null);

  const timeoutRef = React.useRef<number | null>(null);
  const tooltipElRef = React.useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = React.useState<{ left: number; top: number } | null>(null);

  React.useEffect(() => {
    const handleMouseOver = (e: MouseEvent) => {
      const el = (e.target as HTMLElement | null)?.closest<HTMLElement>("[title], [data-tooltip]");
      if (!el) {
        if (timeoutRef.current) clearTimeout(timeoutRef.current);
        setTarget(null);
        setPos(null);
        return;
      }

      // If it's already handled by Radix Tooltip, skip fallback
      if (el.closest("[data-state]")) {
        return;
      }

      // Convert native title to data-tooltip to suppress browser tooltip
      const rawTitle = el.getAttribute("title");
      if (rawTitle) {
        el.setAttribute("data-tooltip", rawTitle);
        el.removeAttribute("title");
      }

      const text = el.getAttribute("data-tooltip");
      if (!text || !text.trim()) {
        if (timeoutRef.current) clearTimeout(timeoutRef.current);
        setTarget(null);
        setPos(null);
        return;
      }

      if (timeoutRef.current) clearTimeout(timeoutRef.current);
      timeoutRef.current = window.setTimeout(() => {
        setTarget({ text, rect: el.getBoundingClientRect() });
      }, 250);
    };

    const handleMouseOut = (e: MouseEvent) => {
      const related = e.relatedTarget as HTMLElement | null;
      const currentEl = (e.target as HTMLElement | null)?.closest<HTMLElement>("[data-tooltip]");
      if (currentEl && related && currentEl.contains(related)) return;

      if (timeoutRef.current) clearTimeout(timeoutRef.current);
      setTarget(null);
      setPos(null);
    };

    const handleHide = () => {
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
      setTarget(null);
      setPos(null);
    };

    window.addEventListener("mouseover", handleMouseOver, { capture: true, passive: true });
    window.addEventListener("mouseout", handleMouseOut, { capture: true, passive: true });
    window.addEventListener("pointerdown", handleHide, { capture: true, passive: true });
    window.addEventListener("scroll", handleHide, { capture: true, passive: true });

    return () => {
      window.removeEventListener("mouseover", handleMouseOver, { capture: true });
      window.removeEventListener("mouseout", handleMouseOut, { capture: true });
      window.removeEventListener("pointerdown", handleHide, { capture: true });
      window.removeEventListener("scroll", handleHide, { capture: true });
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
    };
  }, []);

  // Compute exact position with boundary clamping and zero translate shift
  React.useLayoutEffect(() => {
    if (!target || !tooltipElRef.current) return;
    const el = tooltipElRef.current;
    const width = el.offsetWidth || 100;
    const height = el.offsetHeight || 26;
    const rect = target.rect;

    const side = rect.top < height + 12 ? "bottom" : "top";
    const targetCenterX = rect.left + rect.width / 2;

    const left = Math.max(12, Math.min(window.innerWidth - width - 12, targetCenterX - width / 2));
    const top = side === "top" ? rect.top - height - 6 : rect.bottom + 6;

    setPos({ left, top });
  }, [target]);

  if (!target) return null;

  return (
    <div
      ref={tooltipElRef}
      style={{
        position: "fixed",
        left: pos ? `${pos.left}px` : "-9999px",
        top: pos ? `${pos.top}px` : "-9999px",
        opacity: pos ? 1 : 0,
      }}
      className="pointer-events-none z-[9999] overflow-hidden rounded-md bg-neutral-900 px-2.5 py-1 text-xs text-neutral-50 shadow-md border border-[color-mix(in_srgb,var(--husk-n700)_50%,transparent)] select-none max-w-xs break-words transition-opacity duration-150 ease-out"
    >
      {target.text}
    </div>
  );
}

export { Tooltip, TooltipTrigger, TooltipContent, TooltipProvider };
