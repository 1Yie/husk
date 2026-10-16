import { useState, type ReactNode } from "react";
import { Bin } from "@keyline-icons/react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";

/**
 * Delete button that asks in a modal before `onConfirm` runs.
 *
 * `variant="row"` is the list-row icon button; `variant="text"` is the labelled
 * button the editor dialogs carry in their footer. `confirmTitle` and
 * `confirmDescription` name what is lost, so the prompt says more than "sure?".
 */
export function DeleteButton({
  onConfirm,
  variant = "row",
  label,
  title = "删除",
  confirmTitle,
  confirmDescription = "此操作无法撤销。",
  confirmLabel = "删除",
  busy = false,
  className,
  stopPropagation = false,
}: {
  onConfirm: () => void;
  variant?: "row" | "text";
  /** Label for the `text` variant, e.g. "删除提供商". */
  label?: string;
  title?: string;
  /** Dialog heading; defaults to the button's label or title. */
  confirmTitle?: ReactNode;
  confirmDescription?: ReactNode;
  confirmLabel?: string;
  busy?: boolean;
  className?: string;
  /** Set on rows that are themselves clickable (a row click would otherwise
   *  also toggle them). */
  stopPropagation?: boolean;
}) {
  const [open, setOpen] = useState(false);

  const click = (e: React.MouseEvent) => {
    if (stopPropagation) e.stopPropagation();
    setOpen(true);
  };

  const button =
    variant === "text" ? (
      <Button
        size="sm"
        variant="ghost"
        disabled={busy}
        className={cn("h-8 text-xs text-red-600 hover:text-red-600", className)}
        onClick={click}
      >
        {label ?? "删除"}
      </Button>
    ) : (
      <Button
        variant="ghost"
        size="icon"
        className={cn("h-6 w-6", className)}
        title={title}
        aria-label={title}
        disabled={busy}
        onClick={click}
      >
        <Bin className="h-3.5 w-3.5" />
      </Button>
    );

  return (
    <>
      {button}
      <ConfirmDialog
        open={open}
        onOpenChange={setOpen}
        title={confirmTitle ?? (variant === "text" ? label ?? "删除" : title)}
        description={confirmDescription}
        confirmLabel={confirmLabel}
        onConfirm={onConfirm}
      />
    </>
  );
}
