import { useEffect } from "react";
import { listenUpdateProgress, useUpdateStore } from "./update-store";

/** Let the first screen finish loading before the (silent) network check. */
const STARTUP_DELAY_MS = 4000;

/** Mounted once by the app shell: wires progress events and runs the silent
 * startup check. A failed check is ignored — offline or rate-limited must never
 * bother the user. */
export function useStartupUpdateCheck(enabled: boolean) {
  useEffect(listenUpdateProgress, []);
  useEffect(() => {
    if (!enabled) return;
    const timer = window.setTimeout(() => {
      void useUpdateStore.getState().check({ silent: true });
    }, STARTUP_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [enabled]);
}
