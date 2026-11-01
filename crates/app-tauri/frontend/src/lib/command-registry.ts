/** Command registry — the seam between the global shortcut listener and the
 *  components that own an action. A component registers what it can do; the
 *  listener only knows shortcut actions, never those components.
 */
import { useEffect, useRef } from "react";
import type { ShortcutAction } from "@/lib/shortcuts";

type Runner = () => void;

const runners = new Map<ShortcutAction, Runner>();

/** Register (or replace) the runner for one action; returns its disposer. */
export function registerCommand(action: ShortcutAction, run: Runner): () => void {
  runners.set(action, run);
  return () => {
    if (runners.get(action) === run) runners.delete(action);
  };
}

/** Run the registered command; false when nothing owns it right now. */
export function runCommand(action: ShortcutAction): boolean {
  const run = runners.get(action);
  if (!run) return false;
  run();
  return true;
}

/** Keep a component's runner current without re-registering every render. */
export function useRegisterCommand(action: ShortcutAction, run: Runner) {
  const latest = useRef(run);
  latest.current = run;
  useEffect(
    () => registerCommand(action, () => latest.current()),
    [action],
  );
}
