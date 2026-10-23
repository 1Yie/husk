import * as React from "react";
import * as TooltipPrimitive from "@radix-ui/react-tooltip";

import { cn } from "@/lib/utils";
import { BORDER_ON_BUBBLE, INK_ON_BUBBLE, SURFACE_BUBBLE } from "@/lib/theme";
import { POPPER_PENDING, usePopperPlacedRef } from "@/components/ui/use-popper-placed";

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
        // Always-dark bubble: `SURFACE_BUBBLE` carries the theme-aware surface,
        // the ink stays literal (see lib/theme.ts).
        "z-50 overflow-hidden rounded-md px-2.5 py-1 text-xs shadow-md",
        SURFACE_BUBBLE,
        INK_ON_BUBBLE,
        "animate-in fade-in-0 zoom-in-95",
        !placed && POPPER_PENDING,
        "data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95",
        "data-[side=bottom]:slide-in-from-top-1 data-[side=top]:slide-in-from-bottom-1",
        "data-[side=left]:slide-in-from-right-1 data-[side=right]:slide-in-from-left-1",
        BORDER_ON_BUBBLE,
        "border select-none",
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

/** The bubble floats a few pixels off its trigger, so a pointer heading for it
 *  is briefly over neither element. That gap is what the grace period bridges. */
const HOVER_GRACE_MS = 120;

/**
 * Global fallback tooltip engine:
 * 1. Automatically intercepts any unhandled native `title` attributes across the DOM,
 *    stripping them so the browser never shows its ugly unstyled OS tooltip.
 * 2. Accurately clamps position inside window boundaries so it NEVER clips or gets cut off.
 * 3. Pure opacity fade without diagonal translate displacement.
 * 4. Caps the bubble and scrolls overflowing text inside it, so a multi-kilobyte
 *    title (session body, stack trace, shell command) can no longer paint a
 *    window-sized popup that neither reads nor dismisses.
 */
export function GlobalTooltip() {
  const [target, setTarget] = React.useState<{
    text: string;
    rect: DOMRect;
  } | null>(null);
  /** True only while the text overflows the capped box. A short bubble has to
   *  stay click-through, exactly as it always was. */
  const [scrollable, setScrollable] = React.useState(false);

  const timeoutRef = React.useRef<number | null>(null);
  const hideTimeoutRef = React.useRef<number | null>(null);
  const tooltipElRef = React.useRef<HTMLDivElement | null>(null);
  const scrollableRef = React.useRef(false);
  const [pos, setPos] = React.useState<{ left: number; top: number } | null>(null);

  // The window listeners are bound once; they read the overflow verdict from a ref.
  React.useEffect(() => {
    scrollableRef.current = scrollable;
  }, [scrollable]);

  React.useEffect(() => {
    // `window` dispatches scroll itself, and it is not a Node: `contains` would throw.
    const isInsideBubble = (node: EventTarget | null) =>
      node instanceof Node && !!tooltipElRef.current?.contains(node);

    const cancelClose = () => {
      if (hideTimeoutRef.current) clearTimeout(hideTimeoutRef.current);
      hideTimeoutRef.current = null;
    };

    const hide = () => {
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
      cancelClose();
      setTarget(null);
      setScrollable(false);
      setPos(null);
    };

    /** Leaving a trigger or the bubble closes it after a short grace period —
     *  but only a scrollable bubble, which the pointer may be travelling into.
     *  A short one still closes the instant the pointer leaves. */
    const closeAfterGrace = () => {
      if (!scrollableRef.current) {
        hide();
        return;
      }
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
      cancelClose();
      hideTimeoutRef.current = window.setTimeout(hide, HOVER_GRACE_MS);
    };

    const handleMouseOver = (e: MouseEvent) => {
      // Coming back onto the trigger, or into the bubble, cancels a pending close.
      cancelClose();
      // The bubble is not a descendant of its trigger, so its own hover would
      // otherwise read as "hovered an element without a title".
      if (isInsideBubble(e.target as Node | null)) return;

      const el = (e.target as HTMLElement | null)?.closest<HTMLElement>("[title], [data-tooltip]");
      if (!el) {
        if (timeoutRef.current) clearTimeout(timeoutRef.current);
        setTarget(null);
        setPos(null);
        return;
      }

      // Radix stamps `data-state` on the TRIGGER itself — testing an ancestor disabled
            // the fallback for every Radix-wrapped subtree (the whole chat stream sits in
            // a ContextMenuTrigger, so streamdown's buttons leaked the native bubble).
      if (el.hasAttribute("data-state")) {
        return;
      }

      // Convert native title to data-tooltip to suppress browser tooltip. The
      // strip is one-way on purpose: a title handed back on hide would let the
      // webview paint its own bubble over a pointer that never left the element.
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
        setScrollable(false);
        setTarget({ text, rect: el.getBoundingClientRect() });
      }, 250);
    };

    const handleMouseOut = (e: MouseEvent) => {
      const related = e.relatedTarget as Node | null;
      const from = e.target as Node | null;

      // Moving into the bubble keeps it open.
      if (isInsideBubble(related)) {
        cancelClose();
        return;
      }

      const currentEl = from instanceof Element ? from.closest<HTMLElement>("[data-tooltip]") : null;
      if (currentEl && related && currentEl.contains(related)) return;

      // The 6px gap between trigger and bubble is the bridge the grace covers.
      if (currentEl || isInsideBubble(from)) {
        closeAfterGrace();
        return;
      }

      hide();
    };

    const handlePointerDown = (e: PointerEvent) => {
      // A pointerdown inside the bubble starts a text selection, not a dismissal.
      if (isInsideBubble(e.target as Node | null)) return;
      hide();
    };

    const handleScroll = (e: Event) => {
      // The bubble's own wheel scroll (long text) must not dismiss it; the page
      // scrolling under the anchor still does.
      if (isInsideBubble(e.target as Node | null)) return;
      hide();
    };

    window.addEventListener("mouseover", handleMouseOver, { capture: true, passive: true });
    window.addEventListener("mouseout", handleMouseOut, { capture: true, passive: true });
    window.addEventListener("pointerdown", handlePointerDown, { capture: true, passive: true });
    window.addEventListener("scroll", handleScroll, { capture: true, passive: true });

    return () => {
      window.removeEventListener("mouseover", handleMouseOver, { capture: true });
      window.removeEventListener("mouseout", handleMouseOut, { capture: true });
      window.removeEventListener("pointerdown", handlePointerDown, { capture: true });
      window.removeEventListener("scroll", handleScroll, { capture: true });
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
      if (hideTimeoutRef.current) clearTimeout(hideTimeoutRef.current);
    };
  }, []);

  // Compute exact position with boundary clamping and zero translate shift.
  // `offsetHeight` already reflects the max-height, so flip and clamp measure
  // the box that is actually painted.
  React.useLayoutEffect(() => {
    if (!target || !tooltipElRef.current) return;
    const el = tooltipElRef.current;
    const width = el.offsetWidth || 100;
    const height = el.offsetHeight || 26;
    const rect = target.rect;

    const side = rect.top < height + 12 ? "bottom" : "top";
    const targetCenterX = rect.left + rect.width / 2;

    const left = Math.max(12, Math.min(window.innerWidth - width - 12, targetCenterX - width / 2));
    let top = side === "top" ? rect.top - height - 6 : rect.bottom + 6;

    // A capped bubble can be nearly a window tall, so a side that always fitted
    // a short bubble now has to be pulled back inside the window.
    if (top + height > window.innerHeight - 8) top = Math.max(8, window.innerHeight - height - 8);
    if (top < 8) top = 8;

    setPos({ left, top });
    // Only overflowing text may take the pointer: measuring the capped box is
    // exactly what decides whether there is anything to scroll.
    setScrollable(el.scrollHeight > el.clientHeight);
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
      className={cn(
        "z-[9999] rounded-md px-2.5 py-1 text-xs shadow-md border border-solid select-none max-w-xs break-words transition-opacity duration-150 ease-out",
        // Bounded bubble: long titles scroll inside it instead of covering the window.
        "max-h-[min(20rem,calc(100vh-16px))] overflow-x-hidden overflow-y-auto overscroll-contain",
        // Pointer access only when there is text to scroll, so an ordinary short
        // tooltip never sits between the pointer and the app.
        scrollable ? "pointer-events-auto select-text" : "pointer-events-none",
        SURFACE_BUBBLE,
        INK_ON_BUBBLE,
        BORDER_ON_BUBBLE,
      )}
    >
      {target.text}
    </div>
  );
}

export { Tooltip, TooltipTrigger, TooltipContent, TooltipProvider };
