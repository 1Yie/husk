import { describe, expect, test } from "vitest";
import { normalizeThinkingLevel } from "@/lib/thinking";
import { opensTurn, railHistoryBoundary } from "../rail-history";

describe("model-bound thinking levels", () => {
  test("unsupported restored levels use this model's fallback", () => {
    const levels = ["off", "low", "high", "max"];
    expect(normalizeThinkingLevel("xhigh", levels)).toBe("low");
    expect(normalizeThinkingLevel("high", levels)).toBe("high");
    expect(normalizeThinkingLevel("off", levels)).toBe("off");
    expect(normalizeThinkingLevel(" HIGH ", levels)).toBe("high");
    expect(normalizeThinkingLevel(undefined, levels)).toBe("low");
    expect(normalizeThinkingLevel("xhigh", ["off", "medium", "high"])).toBe("medium");
  });
  test("empty, off-only and unrecognized capabilities cannot carry a previous tier", () => {
    expect(normalizeThinkingLevel("xhigh")).toBe("off");
    expect(normalizeThinkingLevel("xhigh", ["off"])).toBe("off");
    expect(normalizeThinkingLevel("custom", ["custom"])).toBe("off");
  });
});

describe("history boundary markers", () => {
  test("empty/live-only history has no missing boundary", () => {
    expect(railHistoryBoundary([], {})).toEqual({ unloaded: 0, boundaryTurn: null });
    expect(railHistoryBoundary([], { historyStart: 10, turnOffset: 1 })).toEqual({ unloaded: 1, boundaryTurn: null });
    expect(railHistoryBoundary([{ userText: "已加载问题", steps: [] }], { historyStart: 10, turnOffset: 1 }).boundaryTurn).toBeNull();
    expect(railHistoryBoundary([{ steps: [] }], { historyStart: 10, turnTotal: 2 }).boundaryTurn).toBeNull();
  });
  const notice = { steps: [{ type: "system" }] };
  const compaction = { steps: [{ type: "compaction" }] };
  const answer = { steps: [{ type: "text" }] };
  const user = { userText: "问题", steps: [{ type: "text" }] };

  test("fully loaded notices and compactions never invent a missing prompt", () => {
    for (const first of [notice, compaction, answer]) {
      const boundary = railHistoryBoundary([first, user], { historyStart: 0, turnOffset: 0, turnTotal: 1 });
      expect(boundary).toEqual({ unloaded: 0, boundaryTurn: null });
    }
    expect(railHistoryBoundary([notice, user], {})).toEqual({ unloaded: 0, boundaryTurn: null });
  });
  test("a real page cut in an answer keeps one missing-prompt boundary", () => {
    const boundary = railHistoryBoundary([answer, user], { historyStart: 5, turnOffset: 2, turnTotal: 3 });
    expect(boundary.unloaded).toBe(2);
    expect(boundary.boundaryTurn).toBe(answer);
  });
  test("a steer bubble is not a turn of its own", () => {
    const steer = { userText: "插话", steer: true, steps: [{ type: "text" }] };
    expect(opensTurn(user)).toBe(true);
    expect(opensTurn(steer)).toBe(false);
    // Two prompts and one steer loaded, three turns in total: one unloaded —
    // counting the steer would have claimed none.
    const boundary = railHistoryBoundary([user, steer, user], { historyStart: 5, turnTotal: 3 });
    expect(boundary.unloaded).toBe(1);
  });
  test.each(["thinking", "tools", "plan"])("a page cut in %s preserves its missing prompt", (type) => {
    const fragment = { steps: [{ type }] };
    expect(railHistoryBoundary([fragment], { historyStart: 10, turnOffset: 2 }).boundaryTurn).toBe(fragment);
  });
  test("a notice at the loaded page edge is content, not a half answer", () => {
    const boundary = railHistoryBoundary([notice, user], { historyStart: 5, turnOffset: 2, turnTotal: 3 });
    expect(boundary).toEqual({ unloaded: 2, boundaryTurn: null });
  });
  test("legacy counts still support real paging and full history wins over stale totals", () => {
    expect(railHistoryBoundary([answer, user], { historyStart: 5, turnTotal: 3 }).boundaryTurn).toBe(answer);
    expect(railHistoryBoundary([answer, user], { historyStart: 0, turnOffset: 2, turnTotal: 3 }))
      .toEqual({ unloaded: 0, boundaryTurn: null });
  });
});
