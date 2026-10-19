// Height-animating wrapper for content whose size changes in place — a list
// growing by "显示更多" or shrinking on "收起". The inner box is measured with
// a ResizeObserver and the outer box transitions to that height, so any
// content change animates without the caller tracking sizes.

import * as React from "react";
import { cn } from "@/lib/utils";

export function AnimatedHeight({
  children,
  className,
  duration = 260,
}: {
  children: React.ReactNode;
  className?: string;
  duration?: number;
}) {
  const inner = React.useRef<HTMLDivElement>(null);
  // `undefined` until the first measurement: the first paint stays at its
  // natural height instead of animating up from 0.
  const [height, setHeight] = React.useState<number | undefined>(undefined);

  React.useLayoutEffect(() => {
    const el = inner.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => setHeight(el.offsetHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  return (
    <div
      className={cn(
        "overflow-hidden transition-[height] ease-[cubic-bezier(0.16,1,0.3,1)] motion-reduce:transition-none",
        className,
      )}
      style={{ height, transitionDuration: `${duration}ms` }}
    >
      <div ref={inner}>{children}</div>
    </div>
  );
}
