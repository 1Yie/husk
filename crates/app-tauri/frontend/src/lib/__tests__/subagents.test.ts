// The subagent capsule's wording: one child is named, several are counted, and
// once all of them ended it says how they ended.

import { describe, expect, test } from "vitest";
import { childLabel, summarizeChildren } from "../subagents";
import type { SessionChild } from "@/features/chat/hooks/running-children";

const child = (childId: number, name: string, status: SessionChild["status"] = "running"): SessionChild => ({
  childId,
  name,
  task: "检查 auth 分支",
  status,
});

describe("subagent capsule", () => {
  test("says nothing while the session never delegated", () => {
    expect(summarizeChildren([])).toBeNull();
  });

  test("names a single child without its run suffix", () => {
    expect(summarizeChildren([child(1, "review #1")])).toEqual({
      text: "子代理 review 运行中",
      count: 1,
      tone: "running",
    });
  });

  test("counts several", () => {
    const s = summarizeChildren([child(1, "review #1"), child(2, "review #2"), child(3, "subagent")]);
    expect(s).toEqual({ text: "3 个子代理运行中", count: 3, tone: "running" });
  });

  test("counts only the ones still out while any are", () => {
    const s = summarizeChildren([child(1, "review #1", "done"), child(2, "review #2")]);
    expect(s).toEqual({ text: "子代理 review 运行中", count: 1, tone: "running" });
  });

  test("all ended well: 已完成", () => {
    const s = summarizeChildren([child(1, "a", "done"), child(2, "b", "done")]);
    expect(s).toEqual({ text: "2 个子代理已完成", count: 2, tone: "ok" });
  });

  test("a failure or a cancel outranks the successes once all ended", () => {
    const s = summarizeChildren([child(1, "a", "done"), child(2, "b", "failed"), child(3, "c", "aborted")]);
    expect(s).toEqual({ text: "2 个子代理未完成", count: 3, tone: "failed" });
  });
});

describe("childLabel", () => {
  test("keeps a name with no suffix as it is", () => {
    expect(childLabel("subagent")).toBe("subagent");
  });

  test("falls back rather than reading empty", () => {
    expect(childLabel(" #2 ")).toBe("子代理");
  });
});
