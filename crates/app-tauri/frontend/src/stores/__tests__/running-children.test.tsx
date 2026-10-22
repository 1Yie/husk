// `useHasRunningChildren` — the sidebar row's own question: is a delegated
// child of THIS session still out?

import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, test } from "vitest";
import { useAgentStore, useHasRunningChildren } from "../agent-store";
import { emptyView, type StreamItem } from "@/features/chat/hooks/stream-view";

const card = (childId: number, content?: string): StreamItem => ({
  kind: "tool",
  name: `review #${childId}`,
  args: "检查 auth 分支",
  parent: "delegate",
  childId,
  ...(content !== undefined ? { content } : {}),
});

function viewWith(items: StreamItem[]) {
  return { ...emptyView(), items };
}

beforeEach(() => {
  useAgentStore.setState(useAgentStore.getInitialState(), true);
});

describe("useHasRunningChildren", () => {
  test("true while a delegated child's report has not landed", () => {
    useAgentStore.getState().loadView("/parked", 2, viewWith([card(1)]));
    const { result } = renderHook(() => useHasRunningChildren("/parked", 2));
    expect(result.current).toBe(true);
  });

  test("false once the report lands", () => {
    useAgentStore.getState().loadView("/parked", 3, viewWith([card(1, "完成")]));
    const { result } = renderHook(() => useHasRunningChildren("/parked", 3));
    expect(result.current).toBe(false);
  });

  test("false for a session with no view, and for plain tool calls", () => {
    useAgentStore.getState().loadView("/parked", 4, viewWith([{ kind: "tool", name: "read", args: "a.ts" }]));
    const missing = renderHook(() => useHasRunningChildren("/parked", 99));
    expect(missing.result.current).toBe(false);
    const plain = renderHook(() => useHasRunningChildren("/parked", 4));
    expect(plain.result.current).toBe(false);
  });

  test("a parked workspace's children do not leak into another root's same id", () => {
    useAgentStore.getState().loadView("/parked", 7, viewWith([card(1)]));
    const here = renderHook(() => useHasRunningChildren("/project", 7));
    expect(here.result.current).toBe(false);
  });
});
