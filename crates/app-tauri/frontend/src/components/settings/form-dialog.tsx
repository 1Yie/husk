import type { ReactNode } from "react";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

/**
 * The editor-dialog shell every settings pane repeats: title, body, and a
 * footer that is either a single submit button or `secondary` (a destructive
 * action) on the left plus submit on the right.
 *
 * Widths stay per-dialog (`md` default, `sm` for the plugin form) so the
 * extraction changes nothing on screen.
 */
export function FormDialog({
  open,
  onOpenChange,
  title,
  children,
  width = "md",
  secondary,
  busy = false,
  disabled = false,
  submitLabel,
  busyLabel = "保存中…",
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: ReactNode;
  children: ReactNode;
  width?: "md" | "sm";
  /** Left-hand footer slot, e.g. an `ArmedDeleteButton` in `text` variant. */
  secondary?: ReactNode;
  busy?: boolean;
  disabled?: boolean;
  submitLabel: string;
  busyLabel?: string;
  onSubmit: () => void;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className={width === "sm" ? "max-w-sm" : "max-w-md"}>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>
        {children}
        <DialogFooter className={secondary ? "flex justify-between sm:justify-between" : undefined}>
          {secondary}
          <Button
            size="sm"
            className="h-8 text-xs"
            disabled={disabled || busy}
            onClick={onSubmit}
          >
            {busy ? busyLabel : submitLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
