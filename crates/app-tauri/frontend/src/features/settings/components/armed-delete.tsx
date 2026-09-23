import { useEffect, useState } from "react";
import { Bin } from "@keyline-icons/react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";

/**
 * Delete button with its own two-step confirm.
 *
 * First click arms it (the label becomes `确认删除？再点一次`), the second one
 * runs `onConfirm`. An arm nobody confirms clears itself after `ms`, so a row
 * can't sit half-pressed and a later click can't delete by surprise — the whole
 * point of keeping this here instead of re-implementing it per pane.
 *
 * `variant="row"` is the list-row icon button; `variant="text"` is the labelled
 * button the editor dialogs carry in their footer. Both keep the colours they
 * had when they lived in the panes.
 */
export function ArmedDeleteButton({
  onConfirm,
  variant = "row",
  label,
  title = "删除",
  busy = false,
  className,
  stopPropagation = false,
  ms = 4000,
}: {
  onConfirm: () => void;
  variant?: "row" | "text";
  /** Label for the `text` variant, e.g. "删除提供商". */
  label?: string;
  title?: string;
  busy?: boolean;
  className?: string;
  /** Set on rows that are themselves clickable (a row click would otherwise
   *  also toggle them). */
  stopPropagation?: boolean;
  ms?: number;
}) {
  const [armed, setArmed] = useState(false);

  useEffect(() => {
    if (!armed) return;
    const t = window.setTimeout(() => setArmed(false), ms);
    return () => window.clearTimeout(t);
  }, [armed, ms]);

  const click = (e: React.MouseEvent) => {
    if (stopPropagation) e.stopPropagation();
    if (!armed) {
      setArmed(true);
      return;
    }
    setArmed(false);
    onConfirm();
  };

  if (variant === "text") {
    return (
      <Button
        size="sm"
        variant="ghost"
        disabled={busy}
        className={cn(
          "h-8 text-xs text-red-600 hover:text-red-600",
          armed && "bg-red-50 hover:bg-red-100",
          className
        )}
        onClick={click}
      >
        {armed ? "确认删除？再点一次" : (label ?? "删除")}
      </Button>
    );
  }

  if (armed) {
    return (
      <Button
        variant="destructive"
        size="sm"
        className={cn("h-6 px-2 text-[11px]", className)}
        onClick={click}
      >
        确认删除？再点一次
      </Button>
    );
  }
  return (
    <Button
      variant="ghost"
      size="icon"
      className={cn("h-6 w-6", className)}
      title={title}
      disabled={busy}
      onClick={click}
    >
      <Bin className="h-3.5 w-3.5" />
    </Button>
  );
}
