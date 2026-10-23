import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
  activeViewNow, initAgentStore, useActiveView, useActiveViewSelector,
  useAgentStore, useHasMoreHistory, viewNow,
} from "../agent-store";
import { emptyView, type BgJob, type StreamItem } from "@/features/chat/hooks/stream-view";
import { hasRunningChildren } from "@/features/chat/hooks/running-children";
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

  test("a buffer the pump minted before the fold is replaced by the record", async () => {
    start();
    // A session streams before anything folded its history — the pump has to
    // buffer it somewhere, or the events land on the floor.
    emit({ UserPrompt: "后台问答" }, 2);
    emit({ StateChanged: "Reasoning" }, 2);
    await flush();
    expect(viewNow("/project", 2)).toMatchObject({ provisional: true });
    expect(useAgentStore.getState().runningKeys.has("/project:2")).toBe(true);

    // The fold lands: the record REPLACES the fragment (a provisional buffer is
    // not a page anyone can trust), while the live status a fold cannot know
    // survives — without it the sidebar orb blinks off until the session's
    // next event.
    act(() => useAgentStore.getState().loadView("/project", 2, {
      ...emptyView(),
      items: [
        { kind: "user", text: "后台问答", hi: 0 },
        { kind: "assistant", text: "已答", streaming: false, hi: 1 },
      ],
    }, { historyStart: 0, historyTotal: 2 }));
    const folded = viewNow("/project", 2)!;
    expect(folded.items.map((it) => it.kind)).toEqual(["user", "assistant"]);
    expect(folded.provisional).toBeUndefined();
    expect(folded.historyStart).toBe(0);
    expect(useAgentStore.getState().runningKeys.has("/project:2")).toBe(true);

    // A real view — one the fold already installed — is never replaced.
    act(() => useAgentStore.getState().loadView("/project", 2, {
      ...emptyView(),
      items: [{ kind: "user", text: "另一页", hi: 0 }],
    }, { historyStart: 0 }));
    expect(viewNow("/project", 2)!.items.map((it) => it.kind)).toEqual(["user", "assistant"]);
  });

  test("a background-job list has nowhere to land before the view exists", () => {
    const jobs: BgJob[] = [{
      id: "bg-1",
      command: "cargo test",
      state: { kind: "running" },
      started_at_ms: 1,
      elapsed_ms: 1000,
      notify: true,
      has_own_sink: false,
    }];
    // The view is folded asynchronously, so a `bg_list` seed that lands first is
    // dropped — which is why the seed waits for the view (see ChatPage).
    useAgentStore.getState().setBgJobs("/project", 1, jobs);
    expect(activeViewNow().bgJobs).toHaveLength(0);
    useAgentStore.getState().loadView("/project", 1, emptyView());
    useAgentStore.getState().setBgJobs("/project", 1, jobs);
    expect(activeViewNow().bgJobs).toHaveLength(1);
    expect(activeViewNow().bgJobsAt).toBeGreaterThan(0);
  });
});

describe("a reopened child card takes its transcript back", () => {
  const card: StreamItem = {
    kind: "tool",
    name: "review #1",
    args: "看鉴权",
    content: "[subagent report — child_id 7 · 2 tool calls, 3s]\n报告",
    ok: true,
    parent: "delegate",
    childId: 7,
  };
  const segments = [
    { kind: "text" as const, text: "找到三处" },
    { kind: "tool" as const, index: 0 },
  ];
  const rows: StreamItem[] = [
    {
      kind: "tool",
      name: "smart_grep",
      args: "auth",
      content: "3 hits",
      ok: true,
      parent: "review #1",
      childId: 7,
    },
  ];

  const load = () => {
    useAgentStore.getState().loadView("/project", 1, {
      ...emptyView(),
      items: [
        { kind: "user", text: "委派子代理" },
        { kind: "tool", name: "delegate", args: "1 task", content: "x", ok: true },
        card,
        { kind: "assistant", text: "完成", streaming: false },
      ],
    });
  };

  test("the rows land right behind the card, once", () => {
    load();
    act(() => useAgentStore.getState().applyChildTranscript("/project", 1, 7, segments, rows));
    const items = viewNow("/project", 1)!.items;
    expect(items.map((it) => it.kind)).toEqual(["user", "tool", "tool", "tool", "assistant"]);
    expect(items[2]).toMatchObject({ kind: "tool", childId: 7, segments, transcriptLoaded: true });
    expect(items[3]).toMatchObject({ kind: "tool", name: "smart_grep", parent: "review #1" });
    const snapshot = viewNow("/project", 1);
    // A second fetch of the same child finds a card that already has it.
    act(() => useAgentStore.getState().applyChildTranscript("/project", 1, 7, segments, rows));
    expect(viewNow("/project", 1)).toBe(snapshot);
    expect(viewNow("/project", 1)!.items).toHaveLength(5);
  });

  test("a child whose card is gone leaves the view untouched", () => {
    load();
    const snapshot = viewNow("/project", 1);
    act(() => useAgentStore.getState().applyChildTranscript("/project", 1, 99, segments, rows));
    expect(viewNow("/project", 1)).toBe(snapshot);
  });

  test("a cancelled card keeps 已中断 while its transcript fills in", () => {
    // A cancelled child's card carries 已中断 as its body, not a report: the
    // transcript arriving must fill its segments/rows without turning the
    // card green or losing the interruption.
    const cancelled: StreamItem = {
      kind: "tool",
      name: "subagent",
      args: "探索仓库",
      content: "已中断",
      ok: false,
      parent: "delegate",
      childId: 3,
    };
    useAgentStore.getState().loadView("/project", 1, { ...emptyView(), items: [cancelled] });
    act(() => useAgentStore.getState().applyChildTranscript("/project", 1, 3, segments, rows));
    expect(viewNow("/project", 1)!.items[0]).toMatchObject({
      content: "已中断",
      ok: false,
      segments,
      transcriptLoaded: true,
    });
  });
});

describe("a subagent still running when the webview reloaded", () => {
  const child = (childId: number, name: string, task: string, waiting?: string) => ({
    childId,
    name,
    task,
    waiting,
  });
  const load = (items: StreamItem[] = [{ kind: "user", text: "委派子代理" }]) =>
    useAgentStore.getState().loadView("/project", 1, { ...emptyView(), items });

  test("its card is seeded from the live registry", () => {
    load();
    act(() => useAgentStore.getState().seedRunningChildren("/project", 1, [child(7, "review #1", "看鉴权")]));
    const items = viewNow("/project", 1)!.items;
    expect(items).toHaveLength(2);
    const card = items[1];
    expect(card).toMatchObject({
      kind: "tool",
      name: "review #1",
      args: "看鉴权",
      parent: "delegate",
      childId: 7,
    });
    // No report yet — that is what keeps it running, and the 子代理 dock entry
    // alive with it.
    if (card.kind !== "tool") throw new Error("the seed must append a tool card");
    expect(card.content).toBeUndefined();
    expect(card.waiting).toBeUndefined();
    expect(card.startedAt).toBeGreaterThan(0);
    expect(hasRunningChildren(viewNow("/project", 1)!)).toBe(true);
  });

  test("seeding twice cannot draw the card twice, but carries the question", () => {
    load();
    act(() => useAgentStore.getState().seedRunningChildren("/project", 1, [child(7, "review #1", "看鉴权")]));
    act(() => useAgentStore.getState().seedRunningChildren("/project", 1, [child(7, "review #1", "看鉴权")]));
    expect(viewNow("/project", 1)!.items).toHaveLength(2);
    act(() =>
      useAgentStore.getState().seedRunningChildren("/project", 1, [
        child(7, "review #1", "看鉴权", "用哪个 crate?"),
      ]),
    );
    const items = viewNow("/project", 1)!.items;
    expect(items).toHaveLength(2);
    expect(items[1]).toMatchObject({ childId: 7, waiting: "用哪个 crate?" });
  });

  test("a card the live stream already drew keeps its body", () => {
    load([
      {
        kind: "tool",
        name: "subagent",
        args: "看鉴权",
        content: "报告",
        ok: true,
        parent: "delegate",
        childId: 7,
      },
    ]);
    act(() => useAgentStore.getState().seedRunningChildren("/project", 1, [child(7, "subagent", "看鉴权")]));
    expect(viewNow("/project", 1)!.items[0]).toMatchObject({ content: "报告", ok: true });
  });

  test("the body it already streamed fills the card without closing it", () => {
    const segments = [{ kind: "text" as const, text: "找到三处" }];
    const rows: StreamItem[] = [
      {
        kind: "tool",
        name: "smart_grep",
        args: "auth",
        content: "3 hits",
        ok: true,
        parent: "review #1",
        childId: 7,
      },
    ];
    load();
    act(() => useAgentStore.getState().seedRunningChildren("/project", 1, [child(7, "review #1", "看鉴权")]));
    act(() =>
      useAgentStore.getState().applyChildTranscript("/project", 1, 7, segments, rows, { keepRunning: true }),
    );
    const items = viewNow("/project", 1)!.items;
    expect(items).toHaveLength(3);
    expect(items[1]).toMatchObject({ childId: 7, segments });
    const card = items[1];
    if (card.kind !== "tool") throw new Error("the card must still be a tool card");
    expect(card.transcriptLoaded).toBeUndefined();
    expect(card.content).toBeUndefined();
    // The live stream owns the card from here: a second snapshot must not
    // splice its rows in behind rows that arrived after it.
    const snapshot = viewNow("/project", 1);
    act(() =>
      useAgentStore.getState().applyChildTranscript("/project", 1, 7, segments, rows, { keepRunning: true }),
    );
    expect(viewNow("/project", 1)).toBe(snapshot);
  });
});
