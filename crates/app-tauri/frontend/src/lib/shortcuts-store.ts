import { create } from "zustand";
import {
  defaultBindings,
  type ShortcutAction,
  SHORTCUT_ACTIONS,
} from "@/lib/shortcuts";

const STORAGE_KEY = "husk.shortcuts.v1";

export type ShortcutBindings = Record<ShortcutAction, string>;

function load(): ShortcutBindings {
  const bindings = defaultBindings();
  try {
    const raw = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}") as unknown;
    if (!raw || typeof raw !== "object") return bindings;
    for (const action of SHORTCUT_ACTIONS) {
      const value = (raw as Record<string, unknown>)[action];
      // `null` is a cleared binding; anything unknown keeps the default.
      if (value === null) bindings[action] = "";
      else if (typeof value === "string") bindings[action] = value;
    }
  } catch {
    // A corrupt row must not cost the user their defaults.
  }
  return bindings;
}

function save(bindings: ShortcutBindings) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(bindings));
    return true;
  } catch {
    return false;
  }
}

interface ShortcutState {
  bindings: ShortcutBindings;
  /** Persist first, then apply, so a failed write leaves the old binding. */
  setBinding: (action: ShortcutAction, binding: string) => boolean;
  clearBinding: (action: ShortcutAction) => boolean;
  resetAll: () => boolean;
}

export const useShortcutStore = create<ShortcutState>((set, get) => ({
  bindings: load(),
  setBinding: (action, binding) => {
    const next = { ...get().bindings, [action]: binding };
    if (!save(next)) return false;
    set({ bindings: next });
    return true;
  },
  clearBinding: (action) => {
    const next = { ...get().bindings, [action]: "" };
    if (!save(next)) return false;
    set({ bindings: next });
    return true;
  },
  resetAll: () => {
    const next = defaultBindings();
    if (!save(next)) return false;
    set({ bindings: next });
    return true;
  },
}));
