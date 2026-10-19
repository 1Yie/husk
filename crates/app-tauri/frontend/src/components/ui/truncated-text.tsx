// One-line text that truncates, with the full text in a tooltip only when clipped.

import { useCallback, useRef, useState } from "react";
import { TooltipSimple } from "@/components/ui/tooltip";

/** A `truncate`d line that reveals the full text in a tooltip ONLY when it is
 *  actually clipped — a row whose text fits stays quiet. The observer attaches
 *  from a callback ref, not `useRef` + an effect, because React recreates the
 *  span when it swaps between the bare and the tooltip-wrapped return.
 *
 *  Radix Tooltip, not a native `title`: the bubble appears only when the row is
 *  actually clipped, and Radix can position it against the row. */
export function TruncatedText({
  text,
  className,
  side = "left",
}: {
  text: string;
  className?: string;
  side?: "top" | "right" | "bottom" | "left";
}) {
  const [clipped, setClipped] = useState(false);
  const roRef = useRef<ResizeObserver | null>(null);
  // Keyed to `text`: a rename arrives through a config re-fetch and swaps the
  // string WITHOUT changing the box, which the observer below cannot see. A
  // changed ref identity makes React detach/reattach, re-running `check`.
  const measure = useCallback(
    (el: HTMLSpanElement | null) => {
      roRef.current?.disconnect();
      roRef.current = null;
      if (!el) return;
      // `truncate` (overflow:hidden + nowrap) makes scrollWidth the full text
      // width and clientWidth the visible box — +1 absorbs subpixel rounding.
      const check = () => setClipped(el.scrollWidth > el.clientWidth + 1);
      check();
      const ro = new ResizeObserver(check);
      ro.observe(el);
      roRef.current = ro;
    },
    [text],
  );

  const line = (
    <span ref={measure} className={className}>
      {text}
    </span>
  );
  if (!clipped) return line;
  // The default `side="left"` aims the bubble sideways so it never covers the
  // rows below the pointer. Radix flips it automatically (avoidCollisions defaults true)
  // when the left edge runs out of room.
  return (
    <TooltipSimple content={text} side={side}>
      {line}
    </TooltipSimple>
  );
}
