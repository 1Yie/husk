import { lazy, Suspense } from "react";
import { useLayoutStore } from "@/app/layout/layout-store";

const TerminalPanel = lazy(() => import("./terminal-panel").then((module) => ({ default: module.TerminalPanel })));

/** A terminal toggle must not render App and its entire chat/sidebar tree. */
export function TerminalDock({ cwd }: { cwd?: string }) {
  const open = useLayoutStore((state) => state.terminalOpen);
  const setOpen = useLayoutStore((state) => state.setTerminalOpen);
  return (
    <Suspense fallback={null}>
      <TerminalPanel open={open} onOpenChange={setOpen} cwd={cwd} />
    </Suspense>
  );
}
