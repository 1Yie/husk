// Audible half of the notification rule — one short cue per state that stops
// the agent loop: the turn ended, it is parked on an approval or a question,
// or it failed.
//
// Synthesised with Web Audio rather than shipped as a clip: no asset to load
// before the first cue, no per-platform file, and the pitch/volume can sit
// below the OS alert so a long session does not drown the desktop. The shell
// owns the switch (`notification_sound`), independent of whether system
// notifications are on — a tray app with toasts muted may still want the tone.

import { CONFIG_CHANGED_EVENT, getDefaultPrefs } from "@/lib/agent-ipc/sessions";

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

let ctx: AudioContext | null = null;

/** `[frequency Hz, start offset s, duration s]` per note. Rising for a finish,
 *  twice-flat for "you are needed", falling for a failure — the shape carries
 *  the meaning when the window is behind another one. */
const PHRASES: Record<Cue, readonly (readonly [number, number, number])[]> = {
  turn: [
    [784, 0, 0.1],
    [1047, 0.09, 0.14],
  ],
  attention: [
    [988, 0, 0.09],
    [988, 0.13, 0.12],
  ],
  error: [
    [523, 0, 0.12],
    [392, 0.12, 0.18],
  ],
};

function audio(): AudioContext | null {
  if (ctx) return ctx;
  if (typeof AudioContext === "undefined") return null;
  try {
    ctx = new AudioContext();
  } catch {
    return null;
  }
  return ctx;
}

/** Play one cue. Silent when switched off, when the host has no Web Audio
 *  (older WebKitGTK builds, jsdom), or when the previous cue just played. */
export function playCue(cue: Cue) {
  if (!enabled) return;
  const now = performance.now();
  if (now - lastPlayed < MIN_GAP_MS) return;
  lastPlayed = now;
  const ac = audio();
  if (!ac) return;
  // A context created before the first gesture starts suspended and resuming
  // is async, so the very first cue may be dropped. Acceptable: by the time an
  // agent runs, the user has typed or clicked at least once.
  if (ac.state === "suspended") void ac.resume().catch(() => {});
  for (const [freq, at, dur] of PHRASES[cue]) {
    const start = ac.currentTime + at;
    const osc = ac.createOscillator();
    const gain = ac.createGain();
    osc.type = "sine";
    osc.frequency.setValueAtTime(freq, start);
    // Short attack, exponential tail: a square edge clicks, and a cue that
    // clicks is worse than no cue.
    gain.gain.setValueAtTime(0.0001, start);
    gain.gain.linearRampToValueAtTime(0.09, start + 0.008);
    gain.gain.exponentialRampToValueAtTime(0.0001, start + dur);
    osc.connect(gain).connect(ac.destination);
    osc.start(start);
    osc.stop(start + dur + 0.02);
  }
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
