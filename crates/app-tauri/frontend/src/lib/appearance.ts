/**
 * Appearance applier — bridges the persisted `appearance.json` config to
 * the DOM. Both windows (main + settings) call `initAppearance()` once at
 * boot; it resolves the effective mode (system listens to
 * `prefers-color-scheme`), toggles `.dark` on <html>, and writes the
 * accent/bg/fg/font CSS custom properties the shell reads.
 */

import { emit, listen } from "@tauri-apps/api/event";
import type { AppearanceConfig } from "../invoke/agent";
import { getAppearance } from "../invoke/agent";

const mq = window.matchMedia("(prefers-color-scheme: dark)");
const APPEARANCE_CHANGED = "appearance://changed";

/** Effective dark flag for a config — `system` resolves against the OS. */
export function isDarkMode(cfg: AppearanceConfig): boolean {
  if (cfg.theme_mode === "dark") return true;
  if (cfg.theme_mode === "light") return false;
  return mq.matches; // "system"
}

/** Contrast 0–100 → a subtle lift on body text weight/saturation. The UI
 *  already uses neutral-900/100 text; contrast nudges the darkest tokens
 *  up a step without touching component markup. */
function applyContrast(v: number) {
  const el = document.documentElement;
  // Map 0–100 onto a gentle scale — 45 is the neutral midpoint.
  const boost = (v - 45) / 55; // -0.82 … +1.0
  el.style.setProperty("--husk-contrast", String(boost));
}

export function applyAppearance(cfg: AppearanceConfig) {
  const root = document.documentElement;
  root.classList.toggle("dark", isDarkMode(cfg));
  root.style.setProperty("--husk-accent", cfg.accent);
  root.style.setProperty("--husk-bg", cfg.background);
  root.style.setProperty("--husk-fg", cfg.foreground);
  root.style.setProperty("--husk-ui-font", cfg.ui_font);
  root.style.setProperty("--husk-code-font", cfg.code_font);
  applyContrast(cfg.contrast);

  // Body inherits the themed surface — the html element paints before
  // React mounts, so the first frame already matches the stored theme.
  document.body.style.backgroundColor = cfg.background;
  document.body.style.color = cfg.foreground;
  document.body.style.fontFamily = cfg.ui_font;
}

let listener: ((e: MediaQueryListEvent) => void) | null = null;
let unlistenChanged: (() => void) | null = null;

/** Cross-window sync — the settings window writes `appearance.json` then
 *  broadcasts `appearance://changed`; every window re-reads + re-applies
 *  so a change lands live in the main window without a restart. */
async function handleAppearanceChanged() {
  try {
    const cfg = await getAppearance();
    applyAppearance(cfg);
  } catch {
    /* ignore */
  }
}

/** Wire the OS theme watcher + the cross-window sync — `system` mode
 *  follows `prefers-color-scheme`; every mode follows `appearance://changed`.
 *  Returns the unsubscribe for hot-reload paths; calling again replaces
 *  the old wiring. */
export function initAppearance(cfg: AppearanceConfig): () => void {
  applyAppearance(cfg);
  if (listener) mq.removeEventListener("change", listener);
  if (unlistenChanged) unlistenChanged();

  if (cfg.theme_mode === "system") {
    const onChange = () => applyAppearance(cfg);
    mq.addEventListener("change", onChange);
    listener = onChange;
  } else {
    listener = null;
  }

  // Re-read + re-apply whenever any window persists a change.
  void listen(APPEARANCE_CHANGED, handleAppearanceChanged).then((u) => {
    unlistenChanged = u;
  });

  return () => {
    if (listener) mq.removeEventListener("change", listener);
    if (unlistenChanged) unlistenChanged();
  };
}

/** Settings window calls this after persisting — broadcasts to every
 *  window (including itself, which is harmless since it already applied). */
export async function broadcastAppearance() {
  try {
    await emit(APPEARANCE_CHANGED, {});
  } catch {
    /* ignore */
  }
}