import { create } from "zustand";

const STORAGE_KEY = "husk.layout.v1";

function loadSidebar(): boolean {
  try {
    return JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}").sidebarOpen !== false;
  } catch {
    return true;
  }
}

interface LayoutState {
  sidebarOpen: boolean;
  terminalOpen: boolean;
  setSidebarOpen: (open: boolean) => void;
  toggleSidebar: () => void;
  setTerminalOpen: (open: boolean) => void;
  toggleTerminal: () => void;
}

export const useLayoutStore = create<LayoutState>((set, get) => ({
  sidebarOpen: loadSidebar(),
  // Opening a shell is a user action, not a persisted startup side effect.
  terminalOpen: false,
  setSidebarOpen: (sidebarOpen) => {
    set({ sidebarOpen });
    try { localStorage.setItem(STORAGE_KEY, JSON.stringify({ sidebarOpen })); } catch { /* optional preference */ }
  },
  toggleSidebar: () => get().setSidebarOpen(!get().sidebarOpen),
  setTerminalOpen: (terminalOpen) => set({ terminalOpen }),
  toggleTerminal: () => set({ terminalOpen: !get().terminalOpen }),
}));
