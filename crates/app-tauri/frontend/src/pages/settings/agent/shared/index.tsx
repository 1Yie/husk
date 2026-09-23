/** Field + the armed-delete timer shared by the agent panes. */
import { useEffect, useRef, type ReactNode } from "react";
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

export function useAutoDisarm(armed: unknown, disarm: () => void, ms = 4000) {
  const disarmRef = useRef(disarm);
  disarmRef.current = disarm;
  useEffect(() => {
    if (!armed) return;
    const t = window.setTimeout(() => disarmRef.current(), ms);
    return () => window.clearTimeout(t);
  }, [armed, ms]);
}
