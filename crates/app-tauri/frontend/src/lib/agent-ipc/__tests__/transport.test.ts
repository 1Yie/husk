import { describe, expect, test, vi } from "vitest";
import {
  answerQuestion, approveSessionTool, cancelTurn, decideTool, enqueue,
  sendPrompt, setModel, setQueued, setThinkingLevel, steer, steerChild,
} from "../commands";
import { onAgentEvent, onNotificationClick } from "../events";
import { getModelInfo } from "../sessions";
import { deferred, emitAgentEvent, emitTauriEvent, listenerCount, mockCommand, modelInfo, tauri } from "@/test/tauri";

describe("Tauri command contract", () => {
  test.each([
    [() => sendPrompt("问题\n第二行"), { Prompt: { text: "问题\n第二行" } }],
    [() => steer("修正方向"), { Steer: { text: "修正方向" } }],
    [() => steerChild(3, "继续"), { SteerChild: { child_id: 3, text: "继续" } }],
    [() => setQueued(["a", "b"]), { SetQueued: { items: ["a", "b"] } }],
    [() => enqueue("下一步"), { Enqueue: { text: "下一步" } }],
    [() => decideTool(7, false), { ToolDecision: { request_id: 7, approved: false } }],
    [() => approveSessionTool("computer"), { ApproveSessionTool: { tool_name: "computer" } }],
    [() => answerQuestion(8, "选项一"), { AnswerQuestion: { request_id: 8, answer: "选项一" } }],
    [() => setModel("provider", "model"), { SetModel: { provider: "provider", model: "model" } }],
    [() => setThinkingLevel("high"), { SetThinkingLevel: { level: "high" } }],
    [() => cancelTurn(), "Cancel"],
  ])("serializes %j through agent_cmd", async (send, cmd) => {
    await (send as () => Promise<unknown>)();
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", { cmd });
  });

  test("native errors reach callers rather than becoming a successful no-op", async () => {
    mockCommand("agent_cmd", () => { throw new Error("会话已经关闭"); });
    await expect(sendPrompt("hello")).rejects.toThrow("会话已经关闭");
  });

  test("model-info response retains provider identity and capabilities", async () => {
    const info = modelInfo();
    mockCommand("agent_session", () => info, "model_info");
    await expect(getModelInfo()).resolves.toEqual(info);
    expect(tauri.invoke).toHaveBeenCalledWith("agent_session", { op: "model_info" });
  });
});

test("event envelopes preserve workspace/session routing and stop after unsubscribe", async () => {
  const handler = vi.fn();
  const stop = await onAgentEvent(handler);
  const envelope = { root: "/parked", session: 5, event: { UserPrompt: "后台问题" } };
  emitAgentEvent(envelope);
  expect(handler).toHaveBeenCalledExactlyOnceWith(envelope);
  stop();
  expect(listenerCount()).toBe(0);
  emitAgentEvent(envelope);
  expect(handler).toHaveBeenCalledTimes(1);
});

describe("notification activation", () => {
  const event = "husk://notification-click";
  type Click = { id: number; target: { root: string; session: number } };

  function nativeQueue(clicks: Click[] = []) {
    mockCommand("agent_session", () => clicks[0] ?? null, "peek_notification_click");
    mockCommand("agent_session", (args) => {
      const id = (args?.payload as { id: number }).id;
      if (clicks[0]?.id === id) clicks.shift();
      return null;
    }, "ack_notification_click");
    return clicks;
  }

  test("drains startup clicks and follows live clicks in order without replaying them", async () => {
    const clicks = nativeQueue([
      { id: 1, target: { root: "/parked/项目", session: 7 } },
      { id: 2, target: { root: "/other", session: 7 } },
    ]);
    const handler = vi.fn();
    const stop = await onNotificationClick(handler);
    expect(handler.mock.calls.map(([target]) => target)).toEqual([
      { root: "/parked/项目", session: 7 }, { root: "/other", session: 7 },
    ]);
    clicks.push({ id: 3, target: { root: "/third", session: 2 } });
    emitTauriEvent(event, clicks[0].target);
    emitTauriEvent(event, clicks[0].target);
    await vi.waitFor(() => expect(clicks).toHaveLength(0));
    expect(handler).toHaveBeenCalledTimes(3);
    expect(handler).toHaveBeenLastCalledWith({ root: "/third", session: 2 });
    stop();
    expect(listenerCount(event)).toBe(0);
  });

  test("an event racing the initial drain cannot open a target twice", async () => {
    const click = { id: 1, target: { root: "/a", session: 1 } };
    nativeQueue([click]);
    const firstPeek = deferred<Click>();
    let reads = 0;
    mockCommand("agent_session", () => ++reads === 1 ? firstPeek.promise : null, "peek_notification_click");
    const handler = vi.fn();
    const subscribed = onNotificationClick(handler);
    await vi.waitFor(() => expect(reads).toBe(1));
    emitTauriEvent(event, click.target);
    firstPeek.resolve(click);
    const stop = await subscribed;
    expect(handler).toHaveBeenCalledExactlyOnceWith(click.target);
    stop();
  });

  test("listener replacement preserves a click whose read was still in flight", async () => {
    const clicks = nativeQueue();
    const oldHandler = vi.fn();
    const stop = await onNotificationClick(oldHandler);
    const peek = deferred<Click>();
    let reads = 0;
    mockCommand("agent_session", () => ++reads === 1 ? peek.promise : clicks[0] ?? null, "peek_notification_click");
    const click = { id: 1, target: { root: "/a", session: 1 } };
    clicks.push(click);
    emitTauriEvent(event, click.target);
    await vi.waitFor(() => expect(reads).toBe(1));
    stop();
    const newHandler = vi.fn();
    const replacement = onNotificationClick(newHandler);
    peek.resolve(click);
    const stopReplacement = await replacement;
    expect(oldHandler).not.toHaveBeenCalled();
    expect(newHandler).toHaveBeenCalledExactlyOnceWith(click.target);
    expect(clicks).toHaveLength(0);
    stopReplacement();
  });

  test("separate project activations await each navigation before starting the next", async () => {
    const clicks = nativeQueue();
    const firstOpen = deferred<void>();
    const handler = vi.fn(async (_target: Click["target"]): Promise<void> => {
      if (handler.mock.calls.length === 1) await firstOpen.promise;
    });
    const stop = await onNotificationClick(handler);
    clicks.push(
      { id: 1, target: { root: "/a", session: 7 } },
      { id: 2, target: { root: "/b", session: 7 } },
    );
    emitTauriEvent(event, null);
    emitTauriEvent(event, null);
    await vi.waitFor(() => expect(handler).toHaveBeenCalledTimes(1));
    expect(clicks).toHaveLength(2);
    firstOpen.resolve();
    await vi.waitFor(() => expect(clicks).toHaveLength(0));
    expect(handler.mock.calls.map(([target]) => target)).toEqual([
      { root: "/a", session: 7 }, { root: "/b", session: 7 },
    ]);
    stop();
  });

  test("aborting the initial subscription leaves its target for the next listener", async () => {
    const click = { id: 1, target: { root: "/a", session: 1 } };
    const clicks = nativeQueue([click]);
    const controller = new AbortController();
    const handler = vi.fn();
    const subscribed = onNotificationClick(handler, controller.signal);
    controller.abort();
    await subscribed;
    expect(handler).not.toHaveBeenCalled();
    expect(clicks).toEqual([click]);
    expect(listenerCount(event)).toBe(0);
    const next = vi.fn();
    const stop = await onNotificationClick(next);
    expect(next).toHaveBeenCalledExactlyOnceWith(click.target);
    stop();
  });

  test("failed startup drain releases its listener", async () => {
    mockCommand("agent_session", () => { throw new Error("IPC failed"); }, "peek_notification_click");
    await expect(onNotificationClick(vi.fn())).rejects.toThrow("IPC failed");
    expect(listenerCount(event)).toBe(0);
  });
});
