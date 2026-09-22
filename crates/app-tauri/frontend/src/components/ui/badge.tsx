import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

const badgeVariants = cva(
  "inline-flex items-center rounded-md border px-2.5 py-0.5 text-xs font-semibold transition-colors focus:outline-none focus:ring-2 focus:ring-neutral-950 focus:ring-offset-2",
  {
    variants: {
      variant: {
        default:
          "border-transparent bg-neutral-900 text-neutral-50 shadow hover:bg-[color-mix(in_srgb,var(--husk-n900)_80%,transparent)]",
        secondary:
          "border-transparent bg-neutral-100 text-neutral-900 hover:bg-[color-mix(in_srgb,var(--husk-n100)_80%,transparent)]",
        destructive:
          "border-transparent bg-red-600 text-neutral-50 dark:text-[var(--husk-black)] shadow hover:bg-red-600/80",
        outline: "text-neutral-950",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  }
);

export interface BadgeProps
  extends React.HTMLAttributes<HTMLDivElement>,
    VariantProps<typeof badgeVariants> {}

function Badge({ className, variant, ...props }: BadgeProps) {
  return (
    <div className={cn(badgeVariants({ variant }), className)} {...props} />
  );
}

export { Badge, badgeVariants };
