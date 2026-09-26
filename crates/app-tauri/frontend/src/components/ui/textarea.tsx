import * as React from "react";

import { cn } from "@/lib/utils";
import { SURFACE_INPUT } from "@/lib/theme";

export interface TextareaProps
  extends React.TextareaHTMLAttributes<HTMLTextAreaElement> {}

const Textarea = React.forwardRef<HTMLTextAreaElement, TextareaProps>(
  ({ className, ...props }, ref) => {
    return (
      <textarea
        className={cn(
          "flex min-h-[60px] w-full rounded-xl border px-3 py-2 text-sm text-neutral-900 shadow-sm transition-[color,box-shadow] placeholder:text-neutral-500 focus-visible:outline-none focus-visible:border-neutral-400 focus-visible:ring-[3px] focus-visible:ring-[color-mix(in_srgb,var(--husk-n950)_12%,transparent)] disabled:cursor-not-allowed disabled:opacity-50",
          SURFACE_INPUT,
          className
        )}
        ref={ref}
        {...props}
      />
    );
  }
);
Textarea.displayName = "Textarea";

export { Textarea };
