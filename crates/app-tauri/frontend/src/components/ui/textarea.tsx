import * as React from "react";

import { cn } from "@/lib/utils";
import { SURFACE_INPUT } from "@/lib/theme";

export interface TextareaProps
  extends React.TextareaHTMLAttributes<HTMLTextAreaElement> {
  /** `box` (default) draws the rounded shell around the field, `none` leaves
   *  every bit of chrome to the caller — a transparent, overlaid editor. */
  chrome?: "box" | "none";
  /** Extra classes for the shell only (its radius, mostly). `className` keeps
   *  styling the field itself, so typography and padding stay per-call. */
  shellClassName?: string;
}

const Textarea = React.forwardRef<HTMLTextAreaElement, TextareaProps>(
  ({
    className,
    chrome = "box",
    shellClassName,
    autoCorrect = "off",
    autoCapitalize = "none",
    spellCheck = false,
    ...props
  }, ref) => {
    const field = (
      <textarea
        autoCorrect={autoCorrect}
        autoCapitalize={autoCapitalize}
        spellCheck={spellCheck}
        className={cn(
          // `overflow-x-hidden`: WebKitGTK 2.54 honors the global
          // ::-webkit-scrollbar styles, and at fractional devicePixelRatio
          // (XWayland) textarea scrollWidth is inflated ~×dpr, so a phantom
          // 6px track sits under the text. Text wraps, so x-scroll is
          // never legitimate here.
          //
          // `resize-none`: the browser's own grabber can't be styled, and
          // inside a rounded border it shows up as a stray square in the
          // corner. Height comes from `rows`/`max-h`, overflow scrolls.
          "block w-full resize-none bg-transparent text-sm text-neutral-900 outline-none placeholder:text-neutral-500 disabled:cursor-not-allowed disabled:opacity-50 overflow-x-hidden",
          chrome === "box" && "min-h-[60px] px-3 py-2",
          className
        )}
        ref={ref}
        {...props}
      />
    );
    if (chrome === "none") return field;
    // The bar belongs to the FIELD while the border and radius belong to this
    // shell: on a single element the bar rides the border and spills over the
    // rounded corner. Split, the shell clips the bar's ends exactly the way
    // the diff and artifact cards do.
    return (
      <div
        className={cn(
          "w-full overflow-hidden rounded-xl border shadow-sm transition-[color,box-shadow] focus-within:border-neutral-400 focus-within:ring-[3px] focus-within:ring-[color-mix(in_srgb,var(--husk-n950)_12%,transparent)]",
          SURFACE_INPUT,
          shellClassName
        )}
      >
        {field}
      </div>
    );
  }
);
Textarea.displayName = "Textarea";

export { Textarea };
