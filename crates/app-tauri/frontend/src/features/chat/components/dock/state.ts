// Right-dock UI state — open, active tab, width. It lives in its own store
// (not in the page) so the dock and its toggle read one source, and it is
// persisted so a manual open/close and the chosen width survive session
// switches and app restarts.

import { create } from "zustand";

/** The dock's views. New surfaces go here — everything the session produces
 *  hangs off this one sidebar, whatever the workspace mode. */
export type DockTab = "changes" | "artifacts";

export const DOCK_MIN_WIDTH = 260;
export const DOCK_MAX_WIDTH = 560;
export const DOCK_DEFAULT_WIDTH = 300;

const STORAGE_KEY = "husk.dock.v1";

interface Persisted {
  open: boolean;
  tab: DockTab;
  width: number;
}

interface DockState extends Persisted {
  setOpen: (open: boolean) => void;
  toggle: () => void;
  setTab: (tab: DockTab) => void;
  setWidth: (width: number) => void;
}

export function clampDockWidth(width: number): number {
  if (!Number.isFinite(width)) return DOCK_DEFAULT_WIDTH;
  return Math.min(DOCK_MAX_WIDTH, Math.max(DOCK_MIN_WIDTH, Math.round(width)));
}

function load(): Persisted {
  const fallback: Persisted = { open: false, tab: "changes", width: DOCK_DEFAULT_WIDTH };
  if (typeof localStorage === "undefined") return fallback;
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return fallback;
    const p = JSON.parse(raw) as Partial<Persisted>;
    return {
      open: p.open === true,
      tab: p.tab === "artifacts" ? "artifacts" : "changes",
      width: clampDockWidth(Number(p.width ?? DOCK_DEFAULT_WIDTH)),
    };
  } catch {
    return fallback;
  }
}

function save(s: Persisted) {
  if (typeof localStorage === "undefined") return;
  try {
    // Only the persisted slice — the setters must never leak into storage.
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ open: s.open, tab: s.tab, width: s.width }),
    );
  } catch {
    // A blocked/full quota must not break the toggle.
  }
}

export const useDockStore = create<DockState>((set, get) => ({
  ...load(),
  setOpen: (open) => {
    set({ open });
    save({ ...get(), open });
  },
  toggle: () => {
    const open = !get().open;
    set({ open });
    save({ ...get(), open });
  },
  setTab: (tab) => {
    set({ tab });
    save({ ...get(), tab });
  },
  setWidth: (width) => {
    const w = clampDockWidth(width);
    set({ width: w });
    save({ ...get(), width: w });
  },
}));
