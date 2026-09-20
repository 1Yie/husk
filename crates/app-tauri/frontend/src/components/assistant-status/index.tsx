import { useState } from "react";
import { Orb } from "../agent-orb";
import { ChevronDown } from "@keyline-icons/react";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";
import { renderWithTwemoji } from "@/lib/twemoji";

export function AssistantStatus({
  mode,
  thinkingText,
}: {
  mode: "reply" | "thought" | "thinking" | "tools" | null;
  thinkingText: string;
}) {
  const [open, setOpen] = useState(mode !== "thought");
  const [seenMode, setSeenMode] = useState(mode);
  if (mode !== seenMode) {
    setSeenMode(mode);
    if (mode === "thought") setOpen(false);
  }
  if (!mode) return null;

  const live = mode === "reply" || mode === "tools" || mode === "thinking";
  const label =
    mode === "reply"
      ? "正在回复"
      : mode === "tools"
        ? "正在调用工具"
        : mode === "thinking"
          ? "思考中"
          : "思考过程";
  const canToggle = Boolean(thinkingText);

  return (
    <Collapsible open={open} onOpenChange={canToggle ? setOpen : undefined} className="w-full min-w-0 select-none my-1">
      <div className={open && thinkingText ? "sticky top-0 z-10 bg-white" : ""}>
        <CollapsibleTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            aria-expanded={canToggle ? open : undefined}
            className="h-7 w-fit gap-1.5 px-1.5 text-xs font-medium text-neutral-500 hover:text-neutral-700"
            disabled={!canToggle}
          >
            <span className="relative w-5 h-5 shrink-0 flex items-center justify-center">
              {(
                [
                  ["reply", "B3"],
                  ["thinking", "S1"],
                  ["tools", "S3"],
                ] as const
              ).map(([item, variant]) => (
                <span
                  key={item}
                  aria-hidden={!(live && mode === item)}
                  className={cn(
                    "absolute inset-0 flex items-center justify-center transition-opacity duration-300 ease-out",
                    live && mode === item ? "opacity-100" : "opacity-0 pointer-events-none"
                  )}
                >
                  <Orb label={label} variant={variant} size={14} className="text-neutral-800" />
                </span>
              ))}
              <span
                className={cn(
                  "absolute inset-0 flex items-center justify-center transition-opacity duration-300 ease-out",
                  live ? "opacity-0 pointer-events-none" : "opacity-100"
                )}
              >
                <ChevronDown
                  className={cn(
                    "h-3.5 w-3.5 text-neutral-500 transition-transform duration-300",
                    open ? "rotate-0" : "-rotate-90"
                  )}
                />
              </span>
            </span>
            <span className="relative">
              {(["正在回复", "正在调用工具", "思考中", "思考过程"] as const).map(
                (item) => {
                  const active = label === item;
                  return (
                    <span
                      key={item}
                      aria-hidden={!active}
                      className={cn(
                        "whitespace-nowrap transition-opacity duration-300 ease-out",
                        active
                          ? "opacity-100"
                          : "pointer-events-none absolute top-0 left-0 opacity-0"
                      )}
                    >
                      {item}
                    </span>
                  );
                }
              )}
            </span>
          </Button>
        </CollapsibleTrigger>
      </div>
      {thinkingText ? (
        <CollapsibleContent className="overflow-hidden data-[state=closed]:animate-collapsible-up data-[state=open]:animate-collapsible-down">
          <div className="text-neutral-400 mt-1 w-full min-w-0 whitespace-pre-wrap text-[13px] leading-relaxed select-text font-normal pl-6">
            {renderWithTwemoji(thinkingText)}
          </div>
        </CollapsibleContent>
      ) : null}
    </Collapsible>
  );
}
