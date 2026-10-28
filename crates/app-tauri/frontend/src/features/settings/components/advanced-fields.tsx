import { useState, type ReactNode } from "react";
import { ChevronRight } from "@keyline-icons/react";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";

/** Settings form disclosure; typography and spacing match the provider editor. */
export function AdvancedFields({ label, children, defaultOpen = false }: {
  label: string;
  children: ReactNode;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <CollapsibleTrigger className="flex w-full cursor-pointer items-center gap-1 text-[11px] font-semibold uppercase tracking-wide text-neutral-500 hover:text-neutral-800">
        <ChevronRight
          aria-hidden="true"
          className={`h-3 w-3 transition-transform motion-reduce:transition-none ${open ? "rotate-90" : ""}`}
        />
        {label}
      </CollapsibleTrigger>
      {/* Leave room for the controls' focus rings inside the clipping boundary. */}
      <CollapsibleContent className="-mx-1">
        <div className="flex flex-col gap-3 px-1 pb-1 pt-3">{children}</div>
      </CollapsibleContent>
    </Collapsible>
  );
}
