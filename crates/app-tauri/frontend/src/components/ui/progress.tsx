import * as React from "react";

import { cn } from "@/lib/utils";

interface ProgressProps extends React.HTMLAttributes<HTMLDivElement> {
  /** 0–100. Omit for a full bar with a sweeping highlight (running, but no measurable progress). */
  value?: number;
}

const Progress = React.forwardRef<HTMLDivElement, ProgressProps>(
  ({ className, value, ...props }, ref) => {
    const indeterminate = value === undefined;
    return (
      <div
        ref={ref}
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={indeterminate ? undefined : Math.round(value)}
        className={cn("relative h-1.5 w-full overflow-hidden rounded-full bg-neutral-200 dark:bg-white/10", className)}
        {...props}
      >
        <div
          className="relative h-full overflow-hidden rounded-full bg-accent transition-[width] duration-200"
          style={{ width: indeterminate ? "100%" : `${Math.min(Math.max(value, 0), 100)}%` }}
        >
          {indeterminate && (
            <span className="absolute inset-y-0 left-0 w-1/3 animate-progress-scan bg-gradient-to-r from-transparent via-[rgba(255,255,255,0.75)] to-transparent" />
          )}
        </div>
      </div>
    );
  },
);
Progress.displayName = "Progress";

export { Progress };
