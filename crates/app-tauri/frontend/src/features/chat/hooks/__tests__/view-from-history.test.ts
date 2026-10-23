import { describe, expect, test, vi } from "vitest";
import {
  applyChangesFeed,
  childTranscriptCard,
  opensUserTurn,
  steerTextOf,
  viewFromHistory,
  viewFromHistoryChunked,
} from "../view-from-history";
import { SUMMARY_TOOLS } from "../apply-event";
import type { ChatMessage } from "@/types";

const history: ChatMessage[] = [
  { role: "system", content: "internal prompt" },
  { role: "user", content: "internal nudge", notice: "hidden" },
  { role: "system", content: "已切换模型", notice: "system" },
  { role: "user", content: "检查文件", ts: 100 },
  { role: "assistant", content: "", reasoning: "先读文件", ts: 150, duration_ms: 20,
    tool_calls: [{ id: "c1", type: "function", function: { name: "smart_read", arguments: '{"path":"src/a.ts"}' } }] },
  { role: "tool", content: "permission denied", tool_call_id: "c1", is_error: true, ts: 200, duration_ms: 30 },
  { role: "assistant", content: "部分答案\n\n*[cancelled by user]*", ts: 250 },
];

describe("persisted history replay", () => {
  test("internal context is hidden and user-visible notices stay standalone", () => {
    const view = viewFromHistory(history, undefined, 40);
    expect(view.items.map((item) => item.kind)).toEqual(["system", "user", "thinking", "tool", "assistant", "system"]);
    expect(view.items[0]).toMatchObject({ text: "已切换模型", standalone: true, hi: 42 });
    expect(view.items[1]).toMatchObject({ text: "检查文件", hi: 43, ts: 100 });
  });

  test("reasoning time, tool error and cancelled output survive reopening", () => {
    const view = viewFromHistory(history);
    expect(view.items[2]).toMatchObject({ kind: "thinking", text: "先读文件", done: true, startedAt: 130, ts: 150 });
    expect(view.items[3]).toMatchObject({ kind: "tool", name: "smart_read", args: "src/a.ts", ok: false, startedAt: 170 });
    expect(view.items[4]).toMatchObject({ text: "部分答案", streaming: false });
    expect(view.items[5]).toMatchObject({ kind: "system", text: "已被用户中断" });
  });

  test("an older snapshot without timing still replays and malformed cards are skipped", () => {
    const view = viewFromHistory([
      { role: "system", notice: "compacted", content: "broken json" },
      { role: "system", notice: "plan", content: "broken json" },
      { role: "user", content: "The user interrupted: 先停止修改" },
      { role: "assistant", content: "[call: smart_read {}]\n已读", reasoning: "推导" },
    ]);
    expect(view.items).toMatchObject([
      { kind: "user", text: "先停止修改", steer: true },
      { kind: "thinking", text: "推导", done: true },
      { kind: "assistant", text: "已读", streaming: false },
    ]);
  });

  test("a reopened session rebuilds its 改动 list from the write results", () => {
    const view = viewFromHistory([
      { role: "user", content: "改一下", ts: 100 },
      {
        role: "assistant",
        content: "",
        ts: 150,
        tool_calls: [
          {
            id: "c9",
            type: "function",
            function: { name: "fuzzy_patch", arguments: '{"path":"src/a.ts"}' },
          },
        ],
      },
      {
        role: "tool",
        tool_call_id: "c9",
        ts: 200,
        content:
          "patched src/a.ts (line 3)\n\n--- a/src/a.ts\n+++ b/src/a.ts\n@@ -1,2 +1,3 @@\n-old\n+new\n+extra\n",
      },
    ]);
    expect(view.changes).toMatchObject([{ path: "src/a.ts", adds: 2, dels: 1 }]);
  });

  test("a real prompt is not marked as a steer", () => {
    const view = viewFromHistory([{ role: "user", content: "检查文件" }]);
    expect(view.items[0]).not.toHaveProperty("steer");
  });

  test("a tagged steer is read from its tag, and the prefix is stripped", () => {
    const view = viewFromHistory([
      { role: "user", content: "The user interrupted: 先别改 config", notice: "steer" },
    ]);
    expect(view.items).toMatchObject([{ kind: "user", text: "先别改 config", steer: true }]);
  });

  test("a hidden row that quotes the prefix stays hidden and is not a steer", () => {
    const view = viewFromHistory([
      { role: "user", content: "The user interrupted: nudge", notice: "hidden" },
    ]);
    expect(view.items).toEqual([]);
    expect(steerTextOf({ role: "user", content: "The user interrupted: x", notice: "hidden" })).toBeNull();
  });

  test("only prompts open a user turn", () => {
    expect(opensUserTurn({ role: "user", content: "检查文件" })).toBe(true);
    expect(opensUserTurn({ role: "user", content: "The user interrupted: x", notice: "steer" })).toBe(false);
    expect(opensUserTurn({ role: "user", content: "The user interrupted: x" })).toBe(false);
    expect(opensUserTurn({ role: "user", content: "nudge", notice: "hidden" })).toBe(false);
    expect(opensUserTurn({ role: "assistant", content: "ok" })).toBe(false);
  });

  test("persisted usage seeds the visible context meter", () => {
    const view = viewFromHistory([], {
      prompt: 120,
      completion: 30,
      context_window: 8000,
      cached: 40,
      cache_write: 12,
    });
    expect(view.usage).toEqual({
      prompt: 120,
      completion: 30,
      contextWindow: 8000,
      cachedTokens: 40,
      cacheWriteTokens: 12,
    });
  });

  test("persisted usage restores the context breakdown", async () => {
    const breakdown = {
      system: 10, tools: 5, user: 3, memory: 0, assistant: 4,
      reasoning: 6, tool_calls: 2, tool_results: 20, other: 0,
    };
    const usage = { prompt: 120, completion: 30, context_window: 8000, breakdown };
    expect(viewFromHistory([], usage).contextBreakdown).toEqual(breakdown);
    vi.useFakeTimers();
    const task = viewFromHistoryChunked([], usage);
    await vi.runAllTimersAsync();
    expect((await task).contextBreakdown).toEqual(breakdown);
    vi.useRealTimers();
  });

  test("time-sliced rebuild matches sync replay across multiple pages", async () => {
    vi.useFakeTimers();
    const messages = Array.from({ length: 60 }, (_, index): ChatMessage => ({ role: index % 2 ? "assistant" : "user", content: `message ${index}` }));
    const task = viewFromHistoryChunked(messages, undefined, 30);
    await vi.runAllTimersAsync();
    await expect(task).resolves.toEqual(viewFromHistory(messages, undefined, 30));
  });
});

describe("changes feed", () => {
  const page: ChatMessage[] = [
    { role: "user", content: "改第二个文件" },
    { role: "assistant", content: "",
      tool_calls: [{ id: "c2", type: "function", function: { name: "fuzzy_patch", arguments: '{"path":"b.ts"}' } }] },
    { role: "tool", tool_call_id: "c2",
      content: "patched b.ts (line 3)\n\n--- a/b.ts\n+++ b/b.ts\n@@ -1 +1 @@\n-old\n+new" },
  ];

  test("the whole log rebuilds files the mounted page no longer carries", () => {
    const view = viewFromHistory(page);
    expect(view.changes.map((c) => c.path)).toEqual(["b.ts"]);

    // `open` returns only the newest page; the earlier write lives only in the
    // feed — without it the panel reopens one file short.
    applyChangesFeed(view, [
      { tool: "fuzzy_patch", args: '{"path":"a.ts"}',
        content: "patched a.ts (line 1)\n\n--- a/a.ts\n+++ b/a.ts\n@@ -1 +1 @@\n-x\n+y" },
      { tool: "fuzzy_patch", args: '{"path":"b.ts"}',
        content: "patched b.ts (line 3)\n\n--- a/b.ts\n+++ b/b.ts\n@@ -1 +1 @@\n-old\n+new" },
    ]);
    expect(view.changes.map((c) => c.path).sort()).toEqual(["a.ts", "b.ts"]);
    // The feed replaces the page-derived list rather than stacking onto it.
    expect(view.changes.find((c) => c.path === "b.ts")?.patches).toHaveLength(1);
  });

  test("the artifacts dock is rebuilt from the same feed, not emptied by it", () => {
    const view = viewFromHistory([
      { role: "user", content: "做个 PPT" },
      { role: "assistant", content: "",
        tool_calls: [{ id: "o1", type: "function", function: { name: "office_create", arguments: '{"file":"q4/deck.pptx"}' } }] },
      { role: "tool", tool_call_id: "o1", content: "created q4/deck.pptx" },
    ]);
    expect(view.artifacts.map((a) => a.path)).toEqual(["q4/deck.pptx"]);

    applyChangesFeed(view, [
      { tool: "web_download", args: '{"path":"q4/assets/logo.png","url":"https://x/logo.png"}', content: "saved" },
      { tool: "office_create", args: '{"file":"q4/deck.pptx"}', content: "created q4/deck.pptx" },
    ]);
    expect(view.artifacts.map((a) => a.path)).toEqual(["q4/assets/logo.png", "q4/deck.pptx"]);
  });
});

test("the feed asks for every tool the summary parsers read", () => {
  for (const tool of ["fuzzy_patch", "apply_patch", "office_create", "web_download", "office_exec"]) {
    expect(SUMMARY_TOOLS).toContain(tool);
  }
});

describe("a delegated child survives reopening", () => {
  const delegation = (content: string, isError?: boolean): ChatMessage[] => [
    { role: "user", content: "委派子代理", ts: 10 },
    {
      role: "assistant",
      content: "",
      ts: 20,
      tool_calls: [
        {
          id: "d1",
          type: "function",
          function: { name: "delegate", arguments: '{"task":"探索仓库"}' },
        },
      ],
    },
    { role: "tool", content, tool_call_id: "d1", ts: 30, is_error: isError },
  ];

  test("the parked report replays as a waiting card with its child_id", () => {
    const view = viewFromHistory(
      delegation("[subagent reviewer (child_id 7) is waiting for you]\n需要确认分支"),
    );
    expect(view.items.find((it) => it.kind === "tool" && it.parent === "delegate")).toMatchObject({
      parent: "delegate",
      childId: 7,
      waiting: "需要确认分支",
      ok: true,
    });
  });

  test("a failed child card reloads red instead of green", () => {
    const view = viewFromHistory(delegation("已中断", true));
    expect(view.items.find((it) => it.kind === "tool")).toMatchObject({ ok: false });
  });

  test("a cancelled child keeps its child_id and reads 已中断", () => {
    // The marker is all the log kept of it — no report — but the id survives,
    // so the card can still fetch the transcript persisted when it was
    // cancelled, and it reads 已中断 (the live card's own wording), not 失败.
    const view = viewFromHistory(delegation("[subagent cancelled — child_id 31]", true));
    expect(view.items.find((it) => it.kind === "tool" && it.parent === "delegate")).toMatchObject({
      childId: 31,
      ok: false,
      content: "已中断",
    });
  });

  test("a cancelled section of a parallel delegation is interrupted, not failed", () => {
    const view = viewFromHistory([
      { role: "user", content: "并行委派", ts: 10 },
      {
        role: "assistant",
        content: "",
        ts: 20,
        tool_calls: [
          {
            id: "d3",
            type: "function",
            function: { name: "delegate", arguments: '{"tasks":["看鉴权","看性能"]}' },
          },
        ],
      },
      {
        role: "tool",
        content:
          "[delegate — 2 children]\n" +
          "── [0] ok · 3s ──\n[subagent report — child_id 41 · 2 tool calls, 3s]\n鉴权没问题\n" +
          "── [1] cancelled ──\n[subagent cancelled — child_id 42]",
        tool_call_id: "d3",
        ts: 30,
        is_error: true,
      },
    ]);
    const cards = view.items.filter((it) => it.kind === "tool" && it.parent === "delegate");
    expect(cards).toMatchObject([
      { childId: 41, ok: true },
      { childId: 42, ok: false, content: "已中断" },
    ]);
  });

  test("an answer_child row replays onto the child's card", () => {
    const view = viewFromHistory([
      ...delegation("[subagent reviewer (child_id 7) is waiting for you]"),
      {
        role: "assistant",
        content: "",
        ts: 40,
        tool_calls: [
          {
            id: "a1",
            type: "function",
            function: { name: "answer_child", arguments: '{"child_id":7,"answer":"用 main"}' },
          },
        ],
      },
      { role: "tool", content: "已回复", tool_call_id: "a1", ts: 50 },
    ]);
    const card = view.items.find((it) => it.kind === "tool" && it.parent === "delegate");
    expect(card).toMatchObject({ childId: 7 });
    if (card?.kind === "tool") expect(card.waiting).toBeUndefined();
    if (card?.kind === "tool") {
      expect(card.segments).toContainEqual({ kind: "text", text: "> 主代理回复：用 main", muted: true });
    }
  });

  test("a finished report's header names the child it came from", () => {
    const view = viewFromHistory(
      delegation("[subagent report — child_id 12 · 4 tool calls, 9s]\n结论：无问题"),
    );
    expect(view.items.find((it) => it.kind === "tool" && it.parent === "delegate")).toMatchObject({
      childId: 12,
      ok: true,
    });
  });

  test("a parallel delegation names each child its sections report", () => {
    const view = viewFromHistory([
      { role: "user", content: "并行委派", ts: 10 },
      {
        role: "assistant",
        content: "",
        ts: 20,
        tool_calls: [
          {
            id: "d2",
            type: "function",
            function: { name: "delegate", arguments: '{"tasks":["看鉴权","看性能"]}' },
          },
        ],
      },
      {
        role: "tool",
        content:
          "[delegate — 2 children]\n" +
          "── [0] ok · 3s ──\n[subagent report — child_id 21 · 2 tool calls, 3s]\n鉴权没问题\n" +
          "── [1] ok · 5s ──\n[subagent report — child_id 22 · 1 tool calls, 5s]\n性能可以",
        tool_call_id: "d2",
        ts: 30,
      },
    ]);
    const cards = view.items.filter((it) => it.kind === "tool" && it.parent === "delegate");
    expect(cards.map((it) => (it.kind === "tool" ? it.childId : null))).toEqual([21, 22]);
  });
});

/** A page that ends mid-delegation: the app was quit while the child ran, so
 *  the `delegate` call never got a tool row. */
const danglingDelegation: ChatMessage[] = [
  { role: "user", content: "让子代理看看鉴权", ts: 10 },
  {
    role: "assistant",
    content: "",
    ts: 20,
    tool_calls: [
      {
        id: "d1",
        type: "function",
        function: { name: "delegate", arguments: '{"task":"探索仓库"}' },
      },
    ],
  },
];

describe("a child that died with the process", () => {
  test("an unanswered delegate call reopens as an interrupted card", () => {
    // Nothing else in the log can draw the child: its card was live-only, and
    // no tool row follows the call. Cheap and honest beats no card at all.
    const view = viewFromHistory(danglingDelegation, undefined, 0, { dangling: true });
    const card = view.items.find((it) => it.kind === "tool" && it.parent === "delegate");
    expect(card).toMatchObject({
      name: "subagent",
      args: "探索仓库",
      content: "已中断",
      ok: false,
    });
    if (card?.kind === "tool") expect(card.childId).toBeUndefined();
  });

  test("a running session, or an older page, leaves it to whoever has the rest", () => {
    // While the turn is live the stream completes the row; an older page's
    // result is in the newer pages already on screen.
    for (const view of [
      viewFromHistory(danglingDelegation, undefined, 0, { dangling: false }),
      viewFromHistory(danglingDelegation),
    ]) {
      expect(view.items.some((it) => it.kind === "tool" && it.parent === "delegate")).toBe(false);
    }
  });

  test("an answered call draws only the card its report already had", () => {
    const view = viewFromHistory(
      [
        ...danglingDelegation,
        {
          role: "tool",
          content: "[subagent report — child_id 4 · 2 tool calls, 3s]\n报告",
          tool_call_id: "d1",
          ts: 30,
        },
      ],
      undefined,
      0,
      { dangling: true },
    );
    const cards = view.items.filter((it) => it.kind === "tool" && it.parent === "delegate");
    expect(cards).toHaveLength(1);
    expect(cards[0]).toMatchObject({ childId: 4, ok: true });
  });

  test("an unanswered answer_child closes the card it resumed", () => {
    const view = viewFromHistory(
      [
        { role: "user", content: "委派子代理", ts: 10 },
        {
          role: "assistant",
          content: "",
          ts: 20,
          tool_calls: [
            {
              id: "d1",
              type: "function",
              function: { name: "delegate", arguments: '{"task":"探索仓库"}' },
            },
          ],
        },
        {
          role: "tool",
          content: "[subagent subagent (child_id 9) is waiting for you]\nQuestion: 用哪个 crate?",
          tool_call_id: "d1",
          ts: 30,
        },
        {
          role: "assistant",
          content: "",
          ts: 40,
          tool_calls: [
            {
              id: "a1",
              type: "function",
              function: { name: "answer_child", arguments: '{"child_id":9,"answer":"korei"}' },
            },
          ],
        },
      ],
      undefined,
      0,
      { dangling: true },
    );
    const cards = view.items.filter((it) => it.kind === "tool" && it.parent === "delegate");
    expect(cards).toHaveLength(1);
    expect(cards[0]).toMatchObject({ childId: 9, content: "已中断", ok: false });
    if (cards[0].kind === "tool") expect(cards[0].waiting).toBeUndefined();
  });
});

describe("a subagent's own history replays onto its card", () => {
  // What `subagent_transcript` returns for a finished child: its system prompt,
  // the task it was handed, one tool round, a steer, the parent's answer.
  const transcript: ChatMessage[] = [
    { role: "system", content: "你是子代理" },
    { role: "user", content: "探索仓库的鉴权分支", ts: 10 },
    {
      role: "assistant",
      content: "",
      ts: 20,
      tool_calls: [
        { id: "t1", type: "function", function: { name: "smart_grep", arguments: '{"pattern":"auth"}' } },
      ],
    },
    { role: "tool", content: "3 hits", tool_call_id: "t1", ts: 30, duration_ms: 5 },
    { role: "assistant", content: "找到三处", ts: 40 },
    { role: "user", content: "The user interrupted: 只看中间件", notice: "steer", ts: 50 },
    { role: "user", content: "[answer from your parent] 用 main 分支", ts: 60 },
    { role: "assistant", content: "结论：中间件没问题", ts: 70 },
  ];

  test("text, tool, steer and parent answer fold in arrival order", () => {
    const card = childTranscriptCard(transcript, "review #1", 9);
    expect(card.rows).toMatchObject([
      {
        kind: "tool",
        name: "smart_grep",
        args: "auth",
        content: "3 hits",
        ok: true,
        parent: "review #1",
        childId: 9,
        startedAt: 25,
      },
    ]);
    // The system row and the task prompt are context — the live card drew
    // neither, and the chip already carries the task.
    expect(card.segments).toEqual([
      { kind: "tool", index: 0 },
      { kind: "text", text: "找到三处" },
      { kind: "text", text: "> 用户插话：只看中间件\n\n> 主代理回复：用 main 分支", muted: true },
      { kind: "text", text: "结论：中间件没问题" },
    ]);
  });

  test("a failed call replays red and the task prompt stays out of the body", () => {
    const card = childTranscriptCard(
      [
        { role: "user", content: "探索仓库" },
        { role: "assistant", content: "[call: bash {}]\n先看目录", ts: 20 },
        { role: "tool", content: "denied", is_error: true, ts: 30 },
      ],
      "subagent",
    );
    expect(card.rows).toMatchObject([{ ok: false, name: "tool", parent: "subagent" }]);
    expect(card.segments).toEqual([
      { kind: "text", text: "先看目录" },
      { kind: "tool", index: 0 },
    ]);
  });
});
