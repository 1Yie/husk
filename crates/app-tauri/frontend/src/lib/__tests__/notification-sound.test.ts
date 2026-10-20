import { beforeEach, describe, expect, test, vi } from "vitest";
import { initNotificationSound, playCue } from "../notification-sound";
import { mockCommand, tauri } from "@/test/tauri";

/** The module throttles on `performance.now` and keeps that clock across the
 *  whole file, so every test picks its own moment. */
let clock = 0;
beforeEach(() => {
  clock = 0;
  vi.spyOn(performance, "now").mockImplementation(() => clock);
});

const cueCalls = () =>
  vi
    .mocked(tauri.invoke)
    .mock.calls.filter(
      ([command, args]) =>
        command === "agent_session" && (args as { op?: string } | undefined)?.op === "play_cue",
    );

/** The pref read is a promise chain — one macrotask turn settles it. */
const settle = () => new Promise((r) => setTimeout(r, 0));

describe("cue playback", () => {
  test("hands the tone to the shell", () => {
    clock = 10_000;
    playCue("turn");
    expect(cueCalls()).toHaveLength(1);
    expect(cueCalls()[0][1]).toEqual({ op: "play_cue", payload: { cue: "turn" } });
  });

  test("a burst of loop-stopping states is one event", () => {
    clock = 20_000;
    playCue("attention");
    playCue("turn");
    expect(cueCalls()).toHaveLength(1);
    clock = 21_000;
    playCue("turn");
    expect(cueCalls()).toHaveLength(2);
  });

  test("the stored switch silences the shell, not the notify path", async () => {
    mockCommand("agent_session", () => ({ notification_sound: false }), "get_default_prefs");
    const stop = initNotificationSound();
    await settle();
    clock = 30_000;
    playCue("turn");
    expect(cueCalls()).toHaveLength(0);
    stop();
    // Back to the default so the module flag does not leak into later tests.
    mockCommand("agent_session", () => ({ notification_sound: true }), "get_default_prefs");
    const back = initNotificationSound();
    await settle();
    back();
  });
});
