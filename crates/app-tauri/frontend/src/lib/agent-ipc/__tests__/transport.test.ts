import { describe, expect, test, vi } from "vitest";
import {
  answerQuestion, approveSessionTool, cancelTurn, decideTool, enqueue,
  sendPrompt, setModel, setQueued, setThinkingLevel, steer, steerChild,
} from "../commands";
import { onAgentEvent } from "../events";
import { getModelInfo } from "../sessions";
import { emitAgentEvent, listenerCount, mockCommand, modelInfo, tauri } from "@/test/tauri";

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
