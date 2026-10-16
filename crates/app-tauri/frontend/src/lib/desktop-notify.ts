/**
 * Desktop (OS) notifications — the delivery half of the notification center.
 *
 * Husk closes to the tray, so the window is routinely not on screen: a badge
 * inside a hidden window tells nobody anything. The store decides WHAT
 * happened; this decides whether the user can already see it, and hands the
 * rest to the shell.
 */

import {
  CONFIG_CHANGED_EVENT,
  getDefaultPrefs,
  notifyDesktop,
  type NotificationTarget,
} from "@/lib/agent-ipc/sessions";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** Mirrors the `desktop_notifications` pref — on until a stored row says
 *  otherwise (an untouched install has no row at all). */
let enabled = true;

/** The shell's own word on window focus. WebKitGTK keeps `document.hidden`
 *  false and `hasFocus()` true for a minimized window, so the DOM alone
 *  reads "in front" exactly when the user is gone. `null` until the first
 *  report — then the DOM is all there is. */
let nativeFocus: boolean | null = null;
const frontListeners = new Set<() => void>();

/** Whether the window is in front of the user. Deliberately permissive when
 *  nothing can answer: a redundant in-app badge beats a toast for something
 *  already on screen. */
export function isAppInFront(): boolean {
  if (document.hidden) return false;
  if (nativeFocus !== null) return nativeFocus;
  return document.hasFocus() !== false;
}

/** Run `fn` whenever the window may have come back in front — native focus,
 *  DOM focus, or a visibility flip. Callers re-check `isAppInFront`. */
export function onAppFront(fn: () => void): () => void {
  frontListeners.add(fn);
  window.addEventListener("focus", fn);
  document.addEventListener("visibilitychange", fn);
  return () => {
    frontListeners.delete(fn);
    window.removeEventListener("focus", fn);
    document.removeEventListener("visibilitychange", fn);
  };
}

export function sendDesktopNotification(n: { title: string; body?: string; target?: NotificationTarget }) {
  if (!enabled) return;
  // A hidden window (close-to-tray, minimized) has no surface at all, so the
  // shell is also asked to draw attention to it. A merely unfocused one is
  // still on screen and gets the toast only.
  void notifyDesktop(n.title, n.body, document.hidden, n.target).catch(() => {});
}

/** Read the stored switch and follow later saves — the settings pane
 *  broadcasts `CONFIG_CHANGED_EVENT` — and track the window's native focus.
 *  Called once at boot. */
export function initDesktopNotify(): () => void {
  const read = () =>
    void getDefaultPrefs()
      .then((p) => {
        enabled = p.desktop_notifications !== false;
      })
      .catch(() => {});
  read();
  window.addEventListener(CONFIG_CHANGED_EVENT, read);
  let disposed = false;
  let unlistenFocus: (() => void) | undefined;
  void Promise.resolve()
    .then(() =>
      getCurrentWindow().onFocusChanged(({ payload: focused }) => {
        nativeFocus = focused;
        if (focused) for (const fn of frontListeners) fn();
      })
    )
    .then((stop) => {
      if (disposed) stop();
      else unlistenFocus = stop;
    })
    .catch(() => {});
  return () => {
    disposed = true;
    unlistenFocus?.();
    nativeFocus = null;
    window.removeEventListener(CONFIG_CHANGED_EVENT, read);
  };
}
