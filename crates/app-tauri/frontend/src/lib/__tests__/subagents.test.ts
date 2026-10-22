// The subagent capsule's wording: one child is named, several are counted.

import { describe, expect, test } from "vitest";
import { childLabel, summarizeChildren } from "../subagents";
import type { RunningChild } from "@/features/chat/hooks/running-children";

const child = (childId: number, name: string, task = "检查 auth 分支"): RunningChild => ({
  childId,
  name,
  task,
});

describe("subagent capsule", () => {
  test("says nothing while no child runs", () => {
    expect(summarizeChildren([])).toBeNull();
  });

  test("names a single child without its run suffix", () => {
    expect(summarizeChildren([child(1, "review #1")])).toEqual({
      text: "子代理 review 运行中",
      count: 1,
    });
  });

  test("counts several", () => {
    const s = summarizeChildren([child(1, "review #1"), child(2, "review #2"), child(3, "subagent")]);
    expect(s).toEqual({ text: "3 个子代理运行中", count: 3 });
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
