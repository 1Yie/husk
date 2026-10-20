import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@/lib/agent-ipc/sessions", () => ({
  checkUpdate: vi.fn(),
  installUpdate: vi.fn(),
  relaunchApp: vi.fn(),
}));

import { checkUpdate, installUpdate, relaunchApp, type InstallMode, type UpdateCheck } from "@/lib/agent-ipc/sessions";
import { canInstall, useUpdateStore } from "../update-store";

const NEWER: UpdateCheck = {
  current: "0.3.0",
  latest: "0.4.0",
  url: "https://github.com/1Yie/husk/releases/tag/v0.4.0",
  notes: "## 新功能",
  is_newer: true,
  asset: { name: "Husk_0.4.0_amd64.deb", size: 1024 },
};
const CURRENT: UpdateCheck = { ...NEWER, latest: "0.3.0", is_newer: false, asset: null };

const store = () => useUpdateStore.getState();

describe("update store", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.resetAllMocks();
    vi.mocked(checkUpdate).mockResolvedValue(NEWER);
    vi.mocked(installUpdate).mockResolvedValue({ mode: "installed" });
    vi.mocked(relaunchApp).mockResolvedValue(undefined);
    useUpdateStore.setState({ state: { status: "idle" }, open: false, snoozed: null, checking: false });
  });

  it("opens the dialog when a newer release exists", async () => {
    await store().check({ silent: true });
    expect(store().state.status).toBe("available");
    expect(store().open).toBe(true);
  });

  it("stays closed when already up to date", async () => {
    vi.mocked(checkUpdate).mockResolvedValue(CURRENT);
    const info = await store().check({ silent: true });
    expect(info?.is_newer).toBe(false);
    expect(store().state.status).toBe("idle");
    expect(store().open).toBe(false);
  });

  it("a silent check swallows failures; a manual one reports them", async () => {
    vi.mocked(checkUpdate).mockRejectedValue(new Error("offline"));
    await expect(store().check({ silent: true })).resolves.toBeNull();
    await expect(store().check()).rejects.toThrow("offline");
    expect(store().open).toBe(false);
    expect(store().checking).toBe(false);
  });

  it("ignoring a version stops the automatic dialog, but a manual check still shows it", async () => {
    await store().check({ silent: true });
    store().ignore();
    expect(store().open).toBe(false);
    expect(localStorage.getItem("husk.update.ignored")).toBe("0.4.0");

    useUpdateStore.setState({ state: { status: "idle" } });
    await store().check({ silent: true });
    expect(store().open).toBe(false);
    await store().check();
    expect(store().open).toBe(true);
  });

  it("a newer release than the ignored one is announced again", async () => {
    localStorage.setItem("husk.update.ignored", "0.3.9");
    await store().check({ silent: true });
    expect(store().open).toBe(true);
  });

  it("'later' only silences the rest of this session", async () => {
    await store().check({ silent: true });
    store().dismiss();
    expect(store().open).toBe(false);
    expect(localStorage.getItem("husk.update.ignored")).toBeNull();
    useUpdateStore.setState({ state: { status: "idle" } });
    await store().check({ silent: true });
    expect(store().open).toBe(false);
  });

  it("installs, then waits in `ready` instead of restarting by itself", async () => {
    await store().check({ silent: true });
    await store().install();
    expect(installUpdate).toHaveBeenCalledTimes(1);
    expect(relaunchApp).not.toHaveBeenCalled();
    expect(store().state.status).toBe("ready");
    expect(store().open).toBe(true);
  });

  it("a download can be closed and the restart prompt comes back", async () => {
    await store().check({ silent: true });
    let finish!: (value: { mode: InstallMode }) => void;
    vi.mocked(installUpdate).mockImplementation(
      () => new Promise<{ mode: InstallMode }>((resolve) => { finish = resolve; }),
    );
    const running = store().install();
    expect(store().state.status).toBe("installing");
    store().dismiss();
    expect(store().open).toBe(false);
    finish({ mode: "installed" });
    await running;
    expect(store().state.status).toBe("ready");
    expect(store().open).toBe(true);
  });

  it("verify and install are not dismissible", async () => {
    await store().check({ silent: true });
    vi.mocked(installUpdate).mockImplementation(() => new Promise<{ mode: InstallMode }>(() => {}));
    void store().install();
    for (const phase of ["verify", "install"] as const) {
      useUpdateStore.setState((s) =>
        s.state.status === "installing"
          ? { state: { ...s.state, progress: { ...s.state.progress, phase } } }
          : s,
      );
      store().dismiss();
      expect(store().open).toBe(true);
    }
  });

  it("a ready update stays open until the restart", async () => {
    await store().check({ silent: true });
    await store().install();
    store().dismiss();
    expect(store().open).toBe(true);
  });

  it("a failed install keeps the error and can be retried", async () => {
    vi.mocked(installUpdate).mockRejectedValueOnce("安装包校验失败");
    await store().check({ silent: true });
    await store().install();
    const failed = store().state;
    expect(failed.status === "failed" && failed.error).toContain("校验失败");
    await store().install();
    expect(store().state.status).toBe("ready");
  });

  it("relaunch does nothing until the update is ready, then asks the backend", async () => {
    await store().check({ silent: true });
    await store().relaunch();
    expect(relaunchApp).not.toHaveBeenCalled();
    await store().install();
    await store().relaunch();
    expect(relaunchApp).toHaveBeenCalledTimes(1);
  });

  it("a ready update is not checked over", async () => {
    await store().check({ silent: true });
    await store().install();
    expect(await store().check()).toBeNull();
    expect(checkUpdate).toHaveBeenCalledTimes(1);
    expect(store().state.status).toBe("ready");
  });

  it("records the install mode and says a staged installer runs on restart", async () => {
    vi.mocked(installUpdate).mockResolvedValue({ mode: "staged" });
    await store().check({ silent: true });
    await store().install();
    const state = store().state;
    expect(state.status === "ready" && state.mode).toBe("staged");
  });

  it("restores a prepared update as ready instead of offering a reinstall", async () => {
    vi.mocked(checkUpdate).mockResolvedValue({ ...NEWER, prepared_mode: "staged" });
    await store().check({ silent: true });
    const state = store().state;
    expect(state.status).toBe("ready");
    expect(state.status === "ready" && state.mode).toBe("staged");
    expect(store().open).toBe(true);
    expect(installUpdate).not.toHaveBeenCalled();
  });

  it("a failed relaunch stays ready with the error so the restart can be retried", async () => {
    await store().check({ silent: true });
    await store().install();
    vi.mocked(relaunchApp).mockRejectedValueOnce("无法启动重启程序");
    await store().relaunch();
    const failed = store().state;
    expect(failed.status).toBe("ready");
    expect(failed.status === "ready" && failed.relaunchError).toContain("无法启动重启程序");
    // Retrying restart does not reinstall.
    await store().relaunch();
    expect(relaunchApp).toHaveBeenCalledTimes(2);
    expect(installUpdate).toHaveBeenCalledTimes(1);
  });

  it("manual_reason turns off the automatic in-app install", () => {
    expect(canInstall(NEWER)).toBe(true);
    expect(canInstall({ ...NEWER, manual_reason: "AppImage 版本无法原地更新" })).toBe(false);
    expect(canInstall({ ...NEWER, asset: null })).toBe(false);
  });

  it("progress events only apply while installing", async () => {
    await store().check({ silent: true });
    const { listen } = await import("@tauri-apps/api/event");
    const { listenUpdateProgress } = await import("../update-store");
    listenUpdateProgress();
    const handler = vi.mocked(listen).mock.calls[0]?.[1] as ((e: { payload: unknown }) => void) | undefined;
    expect(handler).toBeTypeOf("function");
    handler!({ payload: { phase: "download", downloaded: 5, total: 10 } });
    expect(store().state.status).toBe("available");
  });
});
