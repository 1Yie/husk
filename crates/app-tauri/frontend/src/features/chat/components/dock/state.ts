// Right-dock UI state — open, active tab, width. It lives in its own store
// (not in the page) so the dock and its toggle read one source, and it is
// persisted so a manual open/close and the chosen width survive session
// switches and app restarts.

import { create } from "zustand";

/** The dock's views. New surfaces go here — everything the session produces
 *  hangs off this one sidebar, whatever the workspace mode. */
export type DockTab = "changes" | "artifacts" | "jobs" | "subagents";

export const DOCK_MIN_WIDTH = 260;
export const DOCK_MAX_WIDTH = 560;
export const DOCK_DEFAULT_WIDTH = 300;

const STORAGE_KEY = "husk.dock.v1";

/** The file views — what `filesTab` remembers. */
export type FilesTab = Exclude<DockTab, "jobs" | "subagents">;

interface Persisted {
  open: boolean;
  tab: DockTab;
  /** The file view to come back to when a detour (后台 / 子代理) is on screen.
   *  Without it, leaving 后台 always landed on 改动, even for a reader who was
   *  on 产物, and the dock pill's label jumped while 后台 was up. */
  filesTab: FilesTab;
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
  const fallback: Persisted = {
    open: false,
    tab: "changes",
    filesTab: "changes",
    width: DOCK_DEFAULT_WIDTH,
  };
  if (typeof localStorage === "undefined") return fallback;
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return fallback;
    const p = JSON.parse(raw) as Partial<Persisted>;
    const filesTab: FilesTab = p.filesTab === "artifacts" ? "artifacts" : "changes";
    return {
      open: p.open === true,
      tab:
        p.tab === "artifacts" || p.tab === "jobs" || p.tab === "subagents"
          ? p.tab
          : "changes",
      // A snapshot written before `filesTab` existed: the stored tab was the
      // file view whenever it still is one.
      filesTab:
        p.filesTab == null && (p.tab === "artifacts" || p.tab === "changes")
          ? (p.tab as FilesTab)
          : filesTab,
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
      JSON.stringify({ open: s.open, tab: s.tab, filesTab: s.filesTab, width: s.width }),
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
    // 后台 and 子代理 are detours, not file views: neither ever becomes the one
    // the dock pill names or falls back to.
    const filesTab = tab === "jobs" || tab === "subagents" ? get().filesTab : tab;
    set({ tab, filesTab });
    save({ ...get(), tab, filesTab });
  },
  setWidth: (width) => {
    const w = clampDockWidth(width);
    set({ width: w });
    save({ ...get(), width: w });
  },
}));
