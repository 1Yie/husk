import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { playCue } from "../notification-sound";

/** WebKitGTK keeps one playback stream open for the life of a running
 *  AudioContext, so a cue that finished long ago still marks the app as
 *  playing. The fix is to close the context as soon as the phrase is over. */
class FakeAudioContext {
  static opened: FakeAudioContext[] = [];
  /** Initial state of the next context, and how long its resume takes. */
  static startState: AudioContextState = "running";
  static resumeDelayMs = 0;
  state: AudioContextState = "running";
  currentTime = 0;
  destination = {} as AudioDestinationNode;
  closed = false;

  constructor() {
    this.state = FakeAudioContext.startState;
    FakeAudioContext.opened.push(this);
  }

  resume() {
    return new Promise<void>((resolve) => {
      setTimeout(() => {
        this.state = "running";
        resolve();
      }, FakeAudioContext.resumeDelayMs);
    });
  }

  close() {
    this.closed = true;
    this.state = "closed";
    return Promise.resolve();
  }

  createOscillator() {
    const osc = {
      type: "sine" as OscillatorType,
      frequency: { setValueAtTime: () => {} },
      connect: (node: unknown) => node,
      start: () => {},
      stop: () => {},
    };
    return osc;
  }

  createGain() {
    return {
      gain: {
        setValueAtTime: () => {},
        linearRampToValueAtTime: () => {},
        exponentialRampToValueAtTime: () => {},
      },
      connect: (node: unknown) => node,
    };
  }
}

describe("cue hands the audio device back", () => {
  let clock = 0;

  beforeEach(() => {
    FakeAudioContext.opened = [];
    FakeAudioContext.startState = "running";
    FakeAudioContext.resumeDelayMs = 0;
    // Only the timers are faked. `playCue` throttles on `performance.now`, and
    // fake timers do not necessarily move that clock, so each test drives it.
    vi.useFakeTimers();
    vi.spyOn(performance, "now").mockImplementation(() => clock);
    vi.stubGlobal("AudioContext", FakeAudioContext);
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  test("the context closes once the phrase has finished", () => {
    clock += 10_000;
    playCue("turn");
    expect(FakeAudioContext.opened).toHaveLength(1);
    const [ac] = FakeAudioContext.opened;
    // Mid-phrase: the turn cue runs for ~0.23 s.
    vi.advanceTimersByTime(150);
    expect(ac.closed).toBe(false);
    vi.advanceTimersByTime(300);
    expect(ac.closed).toBe(true);
  });

  test("a suspended context is not closed before its resume lets the phrase play", async () => {
    // The notes sit on the context's clock, which only starts on resume; a
    // hand-back timed from the call would cut the tail of a slow resume.
    FakeAudioContext.startState = "suspended";
    FakeAudioContext.resumeDelayMs = 300;
    clock += 10_000;
    playCue("turn");
    const [ac] = FakeAudioContext.opened;
    await vi.advanceTimersByTimeAsync(450);
    expect(ac.state).toBe("running");
    expect(ac.closed).toBe(false);
    await vi.advanceTimersByTimeAsync(400);
    expect(ac.closed).toBe(true);
  });

  test("a later cue opens a fresh context, it does not reuse the closed one", () => {
    clock += 10_000;
    playCue("turn");
    vi.advanceTimersByTime(1_000);
    clock += 5_000;
    playCue("attention");
    expect(FakeAudioContext.opened).toHaveLength(2);
    expect(FakeAudioContext.opened[0].closed).toBe(true);
    vi.advanceTimersByTime(1_000);
    expect(FakeAudioContext.opened[1].closed).toBe(true);
  });
});
