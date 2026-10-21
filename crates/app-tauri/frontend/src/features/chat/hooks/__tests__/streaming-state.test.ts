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
      // The engine parks in this state before it asks — `pendingApprovalOf`
      // reads it as the "something is waiting" signal.
      { StateChanged: { AwaitingToolConfirmation: { tool_name: "fuzzy_patch", diff_summary: "patch preview" } } },
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

  // A degraded/dead round can report all-zero usage; it must not wipe the
  // last real meter.
  const zero = applyEvents(withWrite, [
    { Usage: { prompt_tokens: 0, completion_tokens: 0, context_window: 1_000_000, cached_tokens: 0 } },
  ]);
  expect(zero.usage.prompt).toBe(1050);
  expect(zero.usage.cachedTokens).toBe(900);

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

test("run/task counts advance live and re-base on the committed figure", () => {
  const base = { ...emptyView(), runTotal: 4, taskTotal: 2 };
  const usage: UiEvent = {
    Usage: { prompt_tokens: 10, completion_tokens: 2, context_window: 1000, cached_tokens: 0 },
  };
  const live = applyEvents(base, [
    { ToolCallStarted: { name: "read", args_preview: "{}", parent: null } },
    { ToolCallFinished: { name: "read", ok: true, content: "x", parent: null } },
    // Delegate children run in their own history.
    { ToolCallFinished: { name: "grep", ok: true, content: "y", parent: "delegate", child_id: 1 } },
    usage,
  ] as UiEvent[]);
  expect(live.taskTotal).toBe(3);
  expect(live.runTotal).toBe(5);

  // `/clear` or compaction shrinks the persisted history — the header must
  // follow it down instead of keeping a stale maximum. `turns` rides along so
  // the card's 用户轮次 moves at every commit, not only on reopen.
  const committed = applyEvents(live, [
    { Custom: { kind: "husk:session_counts", payload: { runs: 1, tasks: 0, turns: 2 } } },
  ]);
  expect(committed.runTotal).toBe(1);
  expect(committed.taskTotal).toBe(0);
  expect(committed.turnTotal).toBe(2);

  // A payload without `turns` leaves the figure alone.
  const withoutTurns = applyEvents(live, [
    { Custom: { kind: "husk:session_counts", payload: { runs: 1, tasks: 0 } } },
  ]);
  expect(withoutTurns.runTotal).toBe(1);
  expect(withoutTurns.turnTotal).toBe(live.turnTotal);

  // Snapshots without counts stay unknown (the header falls back to rows).
  const legacy = applyEvents(emptyView(), [usage]);
  expect(legacy.runTotal).toBeUndefined();
});

test("the live run tick is an upper bound; the commit snaps it to real rows", () => {
  // A quiet round whose `final_text` came back empty loops instead of pushing
  // an assistant row, but its `Usage` still lands. There is no per-round
  // "a row was written" event, so the live figure over-counts by those rounds
  // and the commit's authoritative `husk:session_counts` corrects it.
  const usage: UiEvent = {
    Usage: { prompt_tokens: 10, completion_tokens: 0, context_window: 1000, cached_tokens: 0 },
  };
  const view = applyEvents(
    { ...emptyView(), runTotal: 10, taskTotal: 0 },
    [usage, usage] as UiEvent[],
  );
  expect(view.runTotal).toBe(12);
  // Two rounds, one row.
  const committed = applyEvents(view, [
    { Custom: { kind: "husk:session_counts", payload: { runs: 11, tasks: 0 } } },
  ]);
  expect(committed.runTotal).toBe(11);
});

test("a steer is its own event, and retry rewinds past it", () => {
  const live = applyEvents(emptyView(), [
    { UserPrompt: "检查边界条件" },
    { StateChanged: "Reasoning" },
    { UserSteer: "先别改 config" },
  ]);
  const users = live.items.filter((it) => it.kind === "user");
  expect(users).toMatchObject([{ text: "检查边界条件" }, { text: "先别改 config", steer: true }]);
  expect(users[0]).not.toHaveProperty("steer", true);

  // After the turn settles, the next prompt opens a new turn.
  const next = applyEvents(live, [{ StateChanged: "Finished" }, { UserPrompt: "下一步" }]);
  expect(next.items.at(-1)).not.toHaveProperty("steer", true);

  // Retry drops the whole turn — steer included — back to its opening prompt.
  const retried = applyEvents(live, ["TurnRetry"]);
  expect(retried.items.filter((it) => it.kind === "user")).toEqual([]);
});

test("a steer joins the running turn without restarting its clock or draft", () => {
  const before = applyEvents(emptyView(), [
    { UserPrompt: "检查边界条件" },
    { StateChanged: "Reasoning" },
    { TextDelta: { text: "正在看", parent: null } },
  ]);
  const after = applyEvents(before, [{ UserSteer: "先别改 config" }]);
  expect(after.turnStartedAt).toBe(before.turnStartedAt);
  expect(after.turnOpen).toBe(true);
  expect(after.rate).toEqual(before.rate);
});

test("a prompt is a prompt even while a turn is open", () => {
  // Nothing about the open turn makes a `UserPrompt` a steer: the kernel says
  // so with `UserSteer`, so a queued prompt echoed mid-run still opens its turn.
  const live = applyEvents(emptyView(), [
    { UserPrompt: "第一个" },
    { StateChanged: "Reasoning" },
    { UserPrompt: "排队的第二个" },
  ]);
  const users = live.items.filter((it) => it.kind === "user");
  expect(users).toHaveLength(2);
  expect(users[1]).not.toHaveProperty("steer", true);
});

test("a card the turn left behind never becomes the click target", () => {
  // Approval requested, then the turn settles with no `ToolCallFinished` —
  // what a cancel between the request and the verdict does. The engine
  // discards a verdict for a request it is not waiting on, so the strip must
  // not offer the abandoned card as the next click target.
  const asked = applyEvents(emptyView(), [
    { UserPrompt: "改文件" },
    { ToolCallStarted: { name: "fuzzy_patch", args_preview: "a.ts" } },
    { StateChanged: { AwaitingToolConfirmation: { tool_name: "fuzzy_patch", diff_summary: "preview" } } },
    { ApprovalRequested: { request_id: 3, tool_name: "fuzzy_patch", diff: "preview", fuzzy: false } },
  ]);
  expect(pendingApprovalOf(asked)?.requestId).toBe(3);

  const settled = applyEvents(asked, [{ StateChanged: "Finished" }]);
  expect(pendingApprovalOf(settled)).toBeNull();
  // Closed for good, not merely hidden behind the state gate.
  expect(settled.items.find((it) => it.kind === "tool" && it.approval)).toMatchObject({
    approval: { resolved: true, approved: false },
  });
});

test("a verdict is only asked for while the engine is actually waiting", () => {
  // Same buffer, moved on to `ExecutingTool`: the click already landed and
  // the strip must not offer a second one.
  const executing = applyEvents(emptyView(), [
    { UserPrompt: "改文件" },
    { ToolCallStarted: { name: "fuzzy_patch", args_preview: "a.ts" } },
    { StateChanged: { AwaitingToolConfirmation: { tool_name: "fuzzy_patch", diff_summary: "preview" } } },
    { ApprovalRequested: { request_id: 4, tool_name: "fuzzy_patch", diff: "preview", fuzzy: false } },
    { StateChanged: { ExecutingTool: { tool_name: "fuzzy_patch" } } },
  ]);
  expect(pendingApprovalOf(executing)).toBeNull();
});

test("a tool result cannot land on a sibling call that is still running", () => {
  // Two calls open at once: a result is matched by name as well as position,
  // so it closes its own row and leaves the sibling open.
  const view = applyEvents(emptyView(), [
    { ToolCallStarted: { name: "smart_read", args_preview: "a.ts" } },
    { ToolCallStarted: { name: "grep", args_preview: "TODO" } },
    { ToolCallFinished: { name: "smart_read", ok: true, content: "a" } },
  ]);
  const tools = view.items.filter((it) => it.kind === "tool");
  expect(tools[0]).toMatchObject({ name: "smart_read", content: "a" });
  expect(tools[1]).toMatchObject({ name: "grep" });
  expect(tools[1]).not.toHaveProperty("content");
});

test("a cancelled batch leaves no child card spinning", () => {
  // Cancel drops the batch's dispatch: the engine settles the batch card,
  // but its children never get a `ToolCallFinished` of their own. A settled
  // turn has nothing in flight, so they close as interrupted.
  const view = applyEvents(emptyView(), [
    { UserPrompt: "读几个文件" },
    { StateChanged: "Reasoning" },
    { ToolCallStarted: { name: "batch_execute", args_preview: "" } },
    { ToolCallStarted: { name: "smart_read", args_preview: "a.ts", parent: "batch_execute" } },
    { ToolCallStarted: { name: "smart_grep", args_preview: "TODO", parent: "batch_execute" } },
    { ToolCallFinished: { name: "smart_read", ok: true, content: "a", parent: "batch_execute" } },
    { ToolCallFinished: { name: "batch_execute", ok: false, content: "已被用户中断" } },
  ]);
  const open = view.items.filter((it) => it.kind === "tool" && it.content === undefined);
  expect(open.map((it) => it.kind === "tool" && it.name)).toEqual(["smart_grep"]);

  const settled = applyEvents(view, [{ StateChanged: { Failed: "turn cancelled by user" } }]);
  const tools = settled.items.filter((it) => it.kind === "tool");
  expect(tools.every((it) => it.kind === "tool" && it.content !== undefined)).toBe(true);
  // The child that did finish keeps its own result.
  expect(tools.find((it) => it.kind === "tool" && it.name === "smart_read")).toMatchObject({
    ok: true,
    content: "a",
  });
  expect(tools.find((it) => it.kind === "tool" && it.name === "smart_grep")).toMatchObject({ ok: false });
});

test("a mid-turn compaction does not split the answer into two bubbles", () => {
  // Auto-compaction folds its card into the RUNNING turn, right behind the
  // draft it interrupted. Text must extend the draft, not the card, or one
  // reply would read as two bubbles.
  const view = applyEvents(emptyView(), [
    { UserPrompt: "解释一下" },
    { StateChanged: "Reasoning" },
    { TextDelta: { text: "前半段", parent: null } },
    { CompactionStarted: { manual: false } },
    { Compacted: { before_tokens: 900, after_tokens: 100, removed_messages: 3, note: "n", manual: false, context_window: 1000 } },
    { TextDelta: { text: "，后半段。", parent: null } },
  ]);
  const answers = view.items.filter((it) => it.kind === "assistant");
  expect(answers).toHaveLength(1);
  expect(answers[0]).toMatchObject({ text: "前半段，后半段。", streaming: true });
  // The card keeps the position it happened in, between the two halves.
  expect(view.items.map((it) => it.kind)).toEqual(["user", "assistant", "compaction"]);
});
