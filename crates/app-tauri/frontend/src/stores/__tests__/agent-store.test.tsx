import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
  activeViewNow, initAgentStore, useActiveView, useActiveViewSelector,
  useAgentStore, useHasMoreHistory,
} from "../agent-store";
import { emptyView } from "@/features/chat/hooks/stream-view";
import { deferred, emitAgentEvent, listenerCount, mockCommand, modelInfo, tauri } from "@/test/tauri";
import { clearAll, notificationsNow } from "@/lib/notifications";
import { CONFIG_CHANGED_EVENT, type ModelInfo } from "@/lib/agent-ipc/sessions";
import type { UiEvent } from "@/types";

let dispose: (() => void) | undefined;
beforeEach(() => {
  useAgentStore.setState(useAgentStore.getInitialState(), true);
  useAgentStore.setState({ workspaceRoot: "/project", activeId: 1 });
  clearAll();
});
afterEach(() => { dispose?.(); dispose = undefined; });
function emit(event: UiEvent, session = 1, root = "/project") {
  emitAgentEvent({ root, session, event });
}
async function flush() {
  await act(async () => { await vi.advanceTimersByTimeAsync(16); });
}
function start() { vi.useFakeTimers(); dispose = initAgentStore(); }

describe("kernel event to active view", () => {
  test("installs once and commits an ordered burst once per frame", async () => {
    start();
    expect(initAgentStore()).toBe(dispose);
    expect(listenerCount()).toBe(1);
    emit({ UserPrompt: "检查问题" });
    emit({ StateChanged: "Reasoning" });
    emit({ ReasoningDelta: { text: "分析", parent: null } });
    emit({ ReasoningDelta: { text: "边界", parent: null } });
    expect(activeViewNow().items).toHaveLength(0);
    await flush();
    expect(activeViewNow().items).toMatchObject([
      { kind: "user", text: "检查问题" }, { kind: "thinking", text: "分析边界", done: false },
    ]);
    expect(useAgentStore.getState().runningKeys.has("/project:1")).toBe(true);
    const running = useAgentStore.getState().runningKeys;
    emit({ ReasoningDelta: { text: "。", parent: null } });
    await flush();
    expect(useAgentStore.getState().runningKeys).toBe(running);
    emit({ TextDelta: { text: "答", parent: null } });
    emit({ AssistantMessage: "答案" });
    emit({ StateChanged: "Finished" });
    await flush();
    expect(activeViewNow().items.at(-1)).toMatchObject({ kind: "assistant", text: "答案", streaming: false });
    expect(useAgentStore.getState().runningKeys.size).toBe(0);
    expect(activeViewNow().streaming).toBe(false);
  });

  test("same session ids in different workspaces cannot share a buffer", async () => {
    start();
    emit({ UserPrompt: "当前" });
    emit({ UserPrompt: "后台" }, 1, "/parked");
    await flush();
    expect(activeViewNow().items[0]).toMatchObject({ text: "当前" });
    act(() => useAgentStore.getState().setWorkspaceRoot("/parked"));
    expect(activeViewNow().items[0]).toMatchObject({ text: "后台" });
    expect(useAgentStore.getState().activeId).toBe(1);
  });

  test("only what the user cannot see becomes a notification", async () => {
    start();
    // The foreground session's own turn end is already on screen.
    emit({ StateChanged: "Finished" });
    // A session the user is not looking at.
    emit({ StateChanged: "Finished" }, 2);
    // A parked workspace's failure.
    emit({ StateChanged: { Failed: "请求失败" } }, 1, "/parked");
    // A cancel is the user's own doing, not a failure to report.
    emit({ StateChanged: { Failed: "turn cancelled by user" } }, 3);
    await flush();
    expect(notificationsNow().map((n) => [n.kind, n.title, n.root, n.session, n.body])).toEqual([
      ["error", "会话出错", "/parked", 1, "请求失败"],
      ["turn", "会话已完成", "/project", 2, undefined],
    ]);
  });

  test("bringing the window back reads what the foreground session raised", async () => {
    start();
    const focus = vi.spyOn(document, "hasFocus").mockReturnValue(false);
    emit({ StateChanged: "Finished" });
    emit({ StateChanged: "Finished" }, 2);
    expect(notificationsNow().map((n) => [n.session, n.read])).toEqual([
      [2, false],
      [1, false],
    ]);
    // Still away — a focus event the window does not actually hold reads nothing.
    window.dispatchEvent(new Event("focus"));
    expect(notificationsNow().every((n) => !n.read)).toBe(true);
    focus.mockReturnValue(true);
    window.dispatchEvent(new Event("focus"));
    expect(notificationsNow().map((n) => [n.session, n.read])).toEqual([
      [2, false],
      [1, true],
    ]);
  });

  test("disposing cancels queued work, unsubscribes, and supports reinstall", async () => {
    start();
    await Promise.resolve();
    emit({ UserPrompt: "不应提交" });
    dispose?.();
    expect(listenerCount()).toBe(0);
    await flush();
    expect(activeViewNow().items).toHaveLength(0);
    dispose = initAgentStore();
    emit({ UserPrompt: "新订阅" });
    await flush();
    expect(activeViewNow().items[0]).toMatchObject({ text: "新订阅" });
  });

  test("teardown before async listener registration finishes still unsubscribes", async () => {
    start();
    dispose?.();
    await Promise.resolve();
    expect(listenerCount()).toBe(0);
    expect(tauri.listen).toHaveBeenCalledTimes(1);
  });

  test("a narrow selector stays quiet for reasoning but publishes its own changes", async () => {
    useAgentStore.getState().loadView("/project", 1, emptyView());
    let renders = 0;
    const hook = renderHook(() => { renders++; return useActiveViewSelector((view) => view.queuedPrompts.length); });
    start();
    emit({ ReasoningDelta: { text: "思考", parent: null } });
    await flush();
    expect(renders).toBe(1);
    emit({ QueuedPrompts: { items: ["跟进"] } });
    await flush();
    expect(hook.result.current).toBe(1);
    expect(renders).toBe(2);
  });

  test("only the latest model-info response updates context and cost chips", async () => {
    const first = deferred<ModelInfo>();
    const second = deferred<ModelInfo>();
    let calls = 0;
    mockCommand("agent_session", () => (++calls === 1 ? first.promise : second.promise), "model_info");
    start();
    window.dispatchEvent(new Event(CONFIG_CHANGED_EVENT));
    second.resolve({ ...modelInfo(), models: [{ ...modelInfo().models[0], context_window: 64000, cost: { input: 2 } }] });
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    first.resolve({ ...modelInfo(), models: [{ ...modelInfo().models[0], context_window: 8000, cost: { input: 9 } }] });
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    expect(useAgentStore.getState()).toMatchObject({ ctxWindow: 64000, modelCost: { input: 2 } });
  });
});

describe("persisted history and live state", () => {
  test("loading an old snapshot never overwrites newer streamed content", () => {
    const live = { ...emptyView(), items: [{ kind: "assistant" as const, text: "实时内容", streaming: true }] };
    useAgentStore.getState().loadView("/project", 1, live, { historyStart: 4, historyTotal: 10 });
    useAgentStore.getState().loadView("/project", 1, emptyView(), { historyStart: 0 });
    expect(activeViewNow().items[0]).toMatchObject({ text: "实时内容" });
    expect(activeViewNow().historyStart).toBe(4);
  });

  test("seeds persisted usage and run counts into a live view", () => {
    const live = {
      ...emptyView(),
      items: [{ kind: "assistant" as const, text: "实时内容", streaming: false }],
      historyStart: 4,
    };
    useAgentStore.getState().loadView("/project", 1, live, { historyStart: 4 });
    const seeded = {
      ...emptyView(),
      usage: {
        prompt: 123,
        completion: 4,
        contextWindow: 100_000,
        cachedTokens: 100,
        cacheWriteTokens: 10,
      },
      contextBreakdown: {
        system: 1, tools: 5, user: 2, memory: 0, assistant: 3,
        reasoning: 4, tool_calls: 5, tool_results: 6, other: 0,
      },
    };
    useAgentStore.getState().loadView("/project", 1, seeded, {
      historyStart: 0,
      historyTotal: 10,
      turnTotal: 2,
      runTotal: 7,
      taskTotal: 3,
    });
    const v = activeViewNow();
    expect(v.items[0]).toMatchObject({ text: "实时内容" });
    expect(v.usage.prompt).toBe(123);
    expect(v.contextBreakdown?.tools).toBe(5);
    expect(v.historyStart).toBe(4);
    expect(v.runTotal).toBe(7);
    expect(v.taskTotal).toBe(3);

    // A later reopen after /clear carries smaller persisted counts; the live
    // figure (already re-based by events) wins over a stale maximum.
    useAgentStore.getState().loadView("/project", 1, seeded, { runTotal: 1, taskTotal: 0 });
    expect(activeViewNow().runTotal).toBe(7);
  });

  test("a fresh view takes the persisted counts verbatim, even when smaller", () => {
    useAgentStore.getState().loadView("/project", 2, emptyView(), { runTotal: 0, taskTotal: 0 });
    useAgentStore.getState().setActiveId(2);
    expect(activeViewNow().runTotal).toBe(0);
    expect(activeViewNow().taskTotal).toBe(0);
  });

  test("a live buffer never adopts the open page's pagination", () => {
    // A buffer built purely from events has no `historyStart`: its rows are
    // whatever streamed in since the webview attached, which is not
    // necessarily the page `open` returned. Adopting that page's start made
    // the rail claim unloaded history it had never seen and let the next
    // prepend splice in rows that duplicated or skipped real ones.
    useAgentStore.getState().loadView("/project", 1, {
      ...emptyView(),
      items: [{ kind: "assistant" as const, text: "实时", streaming: false }],
    });
    expect(activeViewNow().historyStart).toBeUndefined();
    useAgentStore.getState().loadView("/project", 1, emptyView(), {
      historyStart: 60,
      historyTotal: 140,
      turnOffset: 20,
      runTotal: 9,
    });
    const v = activeViewNow();
    expect(v.items).toHaveLength(1);
    expect(v.historyStart).toBeUndefined();
    expect(v.turnOffset).toBeUndefined();
    // The counts, unlike pagination, are absolute and do seed.
    expect(v.runTotal).toBe(9);
  });

  test("older pages prepend once, keep live rows, and update history availability", () => {
    const live = { kind: "assistant" as const, text: "最新内容", streaming: true, hi: 5 };
    useAgentStore.getState().loadView("/project", 1, { ...emptyView(), items: [live] }, { historyStart: 5, turnOffset: 2 });
    const hook = renderHook(() => ({ view: useActiveView(), hasMore: useHasMoreHistory() }));
    expect(hook.result.current.hasMore).toBe(true);
    const older = [{ kind: "user" as const, text: "旧问题", hi: 0 }];
    act(() => useAgentStore.getState().prependItems("/project", 1, older, 0, 0));
    expect(hook.result.current.view.items).toHaveLength(2);
    expect(hook.result.current.view.items[1]).toBe(live);
    expect(hook.result.current.hasMore).toBe(false);
    const snapshot = activeViewNow();
    act(() => useAgentStore.getState().prependItems("/project", 1, older, 0));
    expect(activeViewNow()).toBe(snapshot);
  });

  test("dropping a running view removes its running key and resets the active selector", () => {
    useAgentStore.getState().loadView("/project", 1, { ...emptyView(), state: "Reasoning", streaming: true });
    expect(useAgentStore.getState().runningKeys.size).toBe(1);
    useAgentStore.getState().dropView("/project", 1);
    expect(useAgentStore.getState().runningKeys.size).toBe(0);
    expect(activeViewNow().items).toHaveLength(0);
    const snapshot = useAgentStore.getState();
    useAgentStore.getState().dropView("/project", 1);
    expect(useAgentStore.getState()).toBe(snapshot);
  });
});
