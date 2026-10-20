// Audible half of the notification rule — one short cue per state that stops
// the agent loop: the turn ended, it is parked on an approval or a question,
// or it failed.
//
// The shell plays it. The webview owns the RULE (which event deserves a tone,
// whether the switch is on, the anti-drumbeat throttle), but playback cannot
// live in the page: a hidden or minimized webview has its audio stalled, so a
// page-side cue rings exactly while the user is already looking — the opposite
// of what a cue is for. The shell stays alive in the tray, so the tone lands
// whether the window is in front or not.
//
// The switch (`notification_sound`) stays here, independent of whether system
// notifications are on — a tray app with toasts muted may still want the tone.

import { CONFIG_CHANGED_EVENT, getDefaultPrefs, playCueSound } from "@/lib/agent-ipc/sessions";

/** Which phrase to play. Deliberately the same vocabulary the notification
 *  rule uses, so a producer maps nothing. */
export type Cue = "turn" | "attention" | "error";

/** Mirrors the `notification_sound` pref — on until a stored row says
 *  otherwise, the same rule as the desktop switch. */
let enabled = true;

/** Two cues inside this window are one event (a failed turn reports both the
 *  failure and the settle, and every session ends its turn). */
const MIN_GAP_MS = 600;
let lastPlayed = 0;

/** Play one cue. Silent when switched off, or when the previous cue just
 *  played; a shell with no audio player is not an error the UI should show. */
export function playCue(cue: Cue) {
  if (!enabled) return;
  const now = performance.now();
  if (now - lastPlayed < MIN_GAP_MS) return;
  lastPlayed = now;
  void playCueSound(cue).catch(() => {});
}

/** Read the stored switch and follow later saves — the settings pane
 *  broadcasts `CONFIG_CHANGED_EVENT`. Called once at boot. */
export function initNotificationSound(): () => void {
  const read = () =>
    void getDefaultPrefs()
      .then((p) => {
        enabled = p.notification_sound !== false;
      })
      .catch(() => {});
  read();
  window.addEventListener(CONFIG_CHANGED_EVENT, read);
  return () => window.removeEventListener(CONFIG_CHANGED_EVENT, read);
}
