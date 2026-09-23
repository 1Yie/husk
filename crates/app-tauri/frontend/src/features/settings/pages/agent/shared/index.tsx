/** `Field` — the label + control row the agent panes' dialogs are built from. */
import type { ReactNode } from "react";
import { Label } from "@/components/ui/label";

export function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <Label className="text-xs text-neutral-600">{label}</Label>
      {children}
    </div>
  );
}
