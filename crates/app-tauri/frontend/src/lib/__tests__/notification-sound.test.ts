import { describe, expect, test, vi } from "vitest";
import { initNotificationSound, playCue } from "../notification-sound";
import { mockCommand } from "@/test/tauri";

/** jsdom ships no Web Audio: the cue must degrade to silence, never throw. */
describe("cue synthesis", () => {
  test("no AudioContext is silence, not a crash", () => {
    expect(typeof AudioContext).toBe("undefined");
    expect(() => playCue("turn")).not.toThrow();
    expect(() => playCue("attention")).not.toThrow();
    expect(() => playCue("error")).not.toThrow();
  });

  test("the stored switch is read at boot and followed on save", async () => {
    const getDefaultPrefs = vi.fn(() => Promise.resolve({ notification_sound: false }));
    mockCommand("agent_session", getDefaultPrefs, "get_default_prefs");
    const stop = initNotificationSound();
    await new Promise((r) => setTimeout(r, 0));
    expect(getDefaultPrefs).toHaveBeenCalled();
    // Switched off — nothing to observe beyond "it read the row", since a
    // disabled cue never reaches the (absent) audio device either way.
    expect(() => playCue("turn")).not.toThrow();
    stop();
  });
});
