import * as React from "react";

import { cn } from "@/lib/utils";
import { SURFACE_INPUT } from "@/lib/theme";

export interface InputProps
  extends React.InputHTMLAttributes<HTMLInputElement> {}

const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className, type, ...props }, ref) => {
    return (
      <input
        type={type}
        className={cn(
          // `ring-neutral-950/10` never emits a color — `neutral-950` maps
          // to a bare `var(--husk-n950)`, and Tailwind can't apply the `/10`
          // alpha to a var, so the ring fell back to preflight's default
          // blue `--tw-ring-color`. `color-mix` bakes the alpha in directly.
          "flex h-9 w-full rounded-xl border px-3 py-1 text-sm text-neutral-900 shadow-sm transition-[color,box-shadow] file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-neutral-500 focus-visible:outline-none focus-visible:border-neutral-400 focus-visible:ring-[3px] focus-visible:ring-[color-mix(in_srgb,var(--husk-n950)_12%,transparent)] disabled:cursor-not-allowed disabled:opacity-50",
          SURFACE_INPUT,
          className
        )}
        ref={ref}
        {...props}
      />
    );
  }
);
Input.displayName = "Input";

export { Input };
