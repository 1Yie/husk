import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import {
  checkUpdate,
  installUpdate,
  relaunchApp,
  type InstallMode,
  type UpdateCheck,
} from "@/lib/agent-ipc/sessions";

export type UpdatePhase = "download" | "verify" | "install";

export interface UpdateProgress {
  phase: UpdatePhase;
  downloaded: number;
  total: number;
}

export type UpdateState =
  | { status: "idle" }
  | { status: "available"; info: UpdateCheck }
  | { status: "installing"; info: UpdateCheck; progress: UpdateProgress }
  /** Ready; waiting for the user to restart into it. `mode: 'staged'` is a
   *  Windows installer that only runs on restart. A failed restart keeps this
   *  state (with `relaunchError`) so it can be retried instead of reinstalled. */
  | { status: "ready"; info: UpdateCheck; mode: InstallMode; relaunchError?: string }
  | { status: "failed"; info: UpdateCheck; error: string };

/** Whether the backend offers an automatic in-app install for this release. */
export function canInstall(src: UpdateCheck): boolean {
  return src.asset !== null && !src.manual_reason;
}

/** The version the user chose to skip: it is never announced automatically
 * again (a manual check from About still shows it), but a newer one is. */
const IGNORED_KEY = "husk.update.ignored";

interface UpdateStore {
  state: UpdateState;
  open: boolean;
  /** "稍后": the version hidden for this app session only (not persisted). */
  snoozed: string | null;
  /** A manual check is running (the About page's button). */
  checking: boolean;
  setOpen: (open: boolean) => void;
  /** `silent`: startup check — failures and "up to date" say nothing, and a
   * version the user already dismissed does not pop up again. */
  check: (opts?: { silent?: boolean }) => Promise<UpdateCheck | null>;
  install: () => Promise<void>;
  relaunch: () => Promise<void>;
  dismiss: () => void;
  /** Skip this version: no automatic dialog for it any more. */
  ignore: () => void;
}

export const useUpdateStore = create<UpdateStore>((set, get) => ({
  state: { status: "idle" },
  open: false,
  snoozed: null,
  checking: false,
  setOpen: (open) => set({ open }),

  check: async ({ silent = false } = {}) => {
    const { status } = get().state;
    if (get().checking || status === "installing" || status === "ready") return null;
    set({ checking: true });
    try {
      const info = await checkUpdate();
      if (info.is_newer && info.latest) {
        if (info.prepared_mode) {
          // A prepared update survived an app restart (Windows staged
          // installer): show it as ready instead of offering a reinstall.
          set({ state: { status: "ready", info, mode: info.prepared_mode }, open: true });
        } else {
          set({ state: { status: "available", info } });
          const skipped =
            localStorage.getItem(IGNORED_KEY) === info.latest || get().snoozed === info.latest;
          if (!silent || !skipped) set({ open: true });
        }
      }
      return info;
    } catch (e) {
      if (!silent) throw e;
      return null;
    } finally {
      set({ checking: false });
    }
  },

  install: async () => {
    const { state } = get();
    if (state.status === "installing" || state.status === "idle") return;
    const { info } = state;
    set({
      state: {
        status: "installing",
        info,
        progress: { phase: "download", downloaded: 0, total: info.asset?.size ?? 0 },
      },
    });
    try {
      const { mode } = await installUpdate();
      // Ready is the restart prompt: bring the dialog back even when the user
      // closed it mid-download — the new version only starts through it.
      set({ state: { status: "ready", info, mode }, open: true });
    } catch (e) {
      set({ state: { status: "failed", info, error: String(e) } });
    }
  },

  relaunch: async () => {
    const { state } = get();
    if (state.status !== "ready") return;
    try {
      await relaunchApp();
    } catch (e) {
      // The backend kept the prepared update; stay ready so the user can
      // retry the restart rather than reinstalling.
      set({ state: { ...state, relaunchError: String(e) } });
    }
  },

  dismiss: () => {
    const { state } = get();
    // A download is background work — closing only hides the progress. Verify,
    // install and the ready-to-restart prompt are not: there is nothing to do
    // while they run, and ready is the one door back into the new version.
    if (state.status === "ready") return;
    if (state.status === "installing" && state.progress.phase !== "download") return;
    // "稍后": only this session. An installed update stays installed; the new
    // version simply starts on the next launch.
    set({ open: false, snoozed: "info" in state ? state.info.latest : get().snoozed });
  },

  ignore: () => {
    const { state } = get();
    if (state.status === "installing" || !("info" in state) || !state.info.latest) return;
    localStorage.setItem(IGNORED_KEY, state.info.latest);
    set({ open: false });
  },
}));

let listening = false;
/** Wire the backend's `update-progress` events into the store (once). */
export function listenUpdateProgress() {
  if (listening) return;
  listening = true;
  void listen<UpdateProgress>("update-progress", (e) =>
    useUpdateStore.setState((s) =>
      s.state.status === "installing" ? { state: { ...s.state, progress: e.payload } } : s,
    ),
  );
}
