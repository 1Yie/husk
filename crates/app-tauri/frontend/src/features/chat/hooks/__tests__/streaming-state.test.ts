import { describe, expect, vi, test } from "vitest";
import { sameComposerView } from "@/features/chat/components/composer-bar/composer-state";
import { applyEvent, applyEvents } from "@/features/chat/hooks/apply-event";
import { emptyView, pendingApprovalOf } from "@/features/chat/hooks/stream-view";
import type { UiEvent } from "@/types";

const delta = (text: string): UiEvent => ({ ReasoningDelta: { text, parent: null } });
const thinkingView = () => applyEvents(emptyView(), [
  { UserPrompt: "检查边界条件" },
  { StateChanged: "Reasoning" },
  delta("首先分析"),
]);

describe("composer streaming boundary", () => {
  test("identical views and same-size unchanged items preserve the composer", () => {
    const view = thinkingView();
    expect(sameComposerView(view, view)).toBe(true);
    expect(sameComposerView(view, { ...view, items: [...view.items] })).toBe(true);
    expect(sameComposerView(view, { ...view, streaming: !view.streaming })).toBe(false);
  });
  test("reasoning and answer text do not invalidate the composer", () => {
    const before = thinkingView();
    const after = applyEvents(before, [delta("，然后验证"), delta("状态转换。")]);
    expect(sameComposerView(before, after)).toBe(true);
    expect(before.items.at(-1)).toMatchObject({ text: "首先分析" });
    expect(after.items.at(-1)).toMatchObject({ text: "首先分析，然后验证状态转换。" });
    const answer = applyEvents(after, [{ TextDelta: { text: "答案", parent: null } }]);
    expect(sameComposerView(after, answer)).toBe(false); // first answer row
    const answer2 = applyEvents(answer, [{ TextDelta: { text: "继续", parent: null } }]);
    expect(sameComposerView(answer, answer2)).toBe(true);
  });

  test("questions, queued prompts and turn completion always invalidate it", () => {
    const before = thinkingView();
    for (const event of [
      { QueuedPrompts: { items: ["下一步"] } },
      { QuestionAsked: { request_id: 1, question: "选择范围", options: [{ label: "当前工作区" }] } },
      { StateChanged: "Finished" },
    ] satisfies UiEvent[]) {
      expect(sameComposerView(before, applyEvents(before, [event]))).toBe(false);
    }
  });

  test("an earlier tool approval is updated while later reasoning streams", () => {
    const before = applyEvents(thinkingView(), [
      { ToolCallStarted: { name: "fuzzy_patch", args_preview: "修改文件" } },
      delta("继续检查"),
    ]);
    const requested = applyEvents(before, [
      { ApprovalRequested: { request_id: 7, tool_name: "fuzzy_patch", diff: "patch preview", fuzzy: false } },
    ]);
    expect(before.items.length).toBe(requested.items.length);
    expect(sameComposerView(before, requested)).toBe(false);
    expect(pendingApprovalOf(before)).toBeNull();
    expect(pendingApprovalOf(requested)?.requestId).toBe(7);
    const resolved = applyEvents(requested, [
      { ToolCallFinished: { name: "fuzzy_patch", ok: true, content: "完成" } },
    ]);
    expect(sameComposerView(requested, resolved)).toBe(false);
    expect(pendingApprovalOf(resolved)).toBeNull();
  });

  test("todo/plan changes and a rewritten user row are not skipped", () => {
    const before = thinkingView();
    const todo = applyEvents(before, [
      { ToolCallStarted: { name: "todo", args_preview: "记录任务" } },
      { ToolCallFinished: { name: "todo", ok: true, content: "[ ] #1 验证", ui_type: "todo" } },
    ]);
    expect(sameComposerView(before, todo)).toBe(false);
    const changed = { ...todo, items: todo.items.map((item) => item.kind === "tool" ? { ...item, content: "[x] #1 验证" } : item) };
    expect(sameComposerView(todo, changed)).toBe(false);
    const plan = { ...before, items: [{ kind: "plan" as const, plan: { summary: "执行计划", steps: [] } }] };
    const revised = { ...plan, items: [{ kind: "plan" as const, plan: { summary: "修订计划", steps: [] } }] };
    expect(sameComposerView(plan, revised)).toBe(false);
    const user = { ...before, items: before.items.map((item) => item.kind === "user" ? { ...item, text: "另一个问题" } : item) };
    expect(sameComposerView(before, user)).toBe(false);
  });
});

test("cache writes ride the product event after their Usage", () => {
  const usage: UiEvent = {
    Usage: {
      prompt_tokens: 1050,
      completion_tokens: 20,
      context_window: 1_000_000,
      cached_tokens: 900,
    },
  };
  const withUsage = applyEvents(emptyView(), [usage]);
  expect(withUsage.usage).toMatchObject({
    prompt: 1050,
    cachedTokens: 900,
    cacheWriteTokens: 0,
  });
  const withWrite = applyEvents(withUsage, [
    { Custom: { kind: "husk:usage_cache_write", payload: 50 } },
  ]);
  expect(withWrite.usage.cacheWriteTokens).toBe(50);

  // A later round without a write event must not inherit the old count.
  const next = applyEvents(withWrite, [usage]);
  expect(next.usage.cacheWriteTokens).toBe(0);
});

test("batched reasoning preserves control order and immutable snapshots", () => {
  vi.spyOn(Date, "now").mockReturnValue(10000);
  const events: UiEvent[] = [
    { UserPrompt: "验证事件顺序" }, { StateChanged: "Reasoning" },
    delta("分析"), delta("问题"),
    { TextDelta: { text: "答", parent: null } },
    delta("迟到的思考"), { AssistantMessage: "答案" },
    { QueuedPrompts: { items: ["后续问题"] } }, { StateChanged: "Finished" },
  ];
  const sequential = events.reduce(applyEvent, emptyView());
  expect(applyEvents(emptyView(), events)).toEqual(sequential);
  for (let split = 0; split <= events.length; split++) {
    const first = applyEvents(emptyView(), events.slice(0, split));
    const snapshot = structuredClone(first);
    const result = applyEvents(first, events.slice(split));
    expect(first).toEqual(snapshot);
    expect(result).toEqual(sequential);
  }
});
