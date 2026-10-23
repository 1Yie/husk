// Every delegated child of a session, finished ones included — and the list
// comes back from the persisted log, which is what keeps the 子代理 view after a
// restart.

import { describe, expect, test } from "vitest";
import { applyEvents } from "@/features/chat/hooks/apply-event";
import { emptyView } from "@/features/chat/hooks/stream-view";
import { viewFromHistory } from "@/features/chat/hooks/view-from-history";
import {
  childrenFromFeed,
  extractChildren,
  sessionChildren,
  withoutDismissed,
} from "@/features/chat/hooks/running-children";
import type { ChatMessage, UiEvent } from "@/types";
import type { ToolFeedRow } from "@/lib/agent-ipc/sessions";

const card = (childId: number): UiEvent => ({
  ToolCallStarted: { name: `review #${childId}`, args_preview: "检查鉴权", parent: "delegate", child_id: childId },
});

describe("extractChildren", () => {
  test("a child that reported stays in the list, now 已完成, with its report", () => {
    const v = applyEvents(emptyView(), [
      { UserPrompt: "委派" },
      { ToolCallStarted: { name: "delegate", args_preview: "1 task" } },
      card(5),
      {
        ToolCallFinished: {
          name: "review #5",
          ok: true,
          content: "[subagent report — child_id 5 · 3 tool calls, 9s]\n结论：无问题",
          parent: "delegate",
          child_id: 5,
        },
      },
    ] as UiEvent[]);
    expect(extractChildren(v)).toEqual([
      { childId: 5, name: "review #5", task: "检查鉴权", status: "done", report: "结论：无问题" },
    ]);
  });

  test("running, failed and cancelled children each read as themselves", () => {
    const v = applyEvents(emptyView(), [
      { UserPrompt: "委派" },
      card(1),
      card(2),
      card(3),
      { ToolCallFinished: { name: "review #2", ok: false, content: "超时", parent: "delegate", child_id: 2 } },
      { ToolCallFinished: { name: "review #3", ok: false, content: "已中断", parent: "delegate", child_id: 3 } },
    ] as UiEvent[]);
    expect(extractChildren(v).map((c) => [c.childId, c.status])).toEqual([
      [1, "running"],
      [2, "failed"],
      [3, "aborted"],
    ]);
  });

  test("a finished child is rebuilt from the persisted log on reopen", () => {
    const history: ChatMessage[] = [
      { role: "user", content: "委派子代理", ts: 10 },
      {
        role: "assistant",
        content: "",
        ts: 20,
        tool_calls: [{ id: "d1", type: "function", function: { name: "delegate", arguments: '{"task":"探索仓库"}' } }],
      },
      {
        role: "tool",
        content: "[subagent report — child_id 12 · 4 tool calls, 9s]\n结论：无问题",
        tool_call_id: "d1",
        ts: 30,
      },
    ];
    expect(extractChildren(viewFromHistory(history))).toMatchObject([
      { childId: 12, status: "done", report: "结论：无问题" },
    ]);
  });

  test("a parked child replays as waiting, not finished", () => {
    const history: ChatMessage[] = [
      { role: "user", content: "委派子代理", ts: 10 },
      {
        role: "assistant",
        content: "",
        ts: 20,
        tool_calls: [{ id: "d1", type: "function", function: { name: "delegate", arguments: '{"task":"探索仓库"}' } }],
      },
      {
        role: "tool",
        content: "[subagent reviewer (child_id 7) is waiting for you]\n需要确认分支",
        tool_call_id: "d1",
        ts: 30,
      },
    ];
    expect(extractChildren(viewFromHistory(history))).toMatchObject([
      { childId: 7, status: "waiting", waiting: "需要确认分支" },
    ]);
  });
});

/** One `tool_rows_feed` row — `index` is its position in the full history. */
const feedRow = (
  index: number,
  tool: string,
  args: string,
  content: string,
  is_error = false,
): ToolFeedRow => ({ tool, args, content, is_error, index });

describe("childrenFromFeed", () => {
  test("a child of an earlier page reads done with its report and its row", () => {
    const rows = [
      feedRow(
        2,
        "delegate",
        '{"task":"探索仓库"}',
        "[subagent report — child_id 12 · 4 tool calls, 9s]\n结论：无问题",
      ),
    ];
    expect(childrenFromFeed(rows)).toEqual([
      {
        childId: 12,
        name: "subagent",
        task: "探索仓库",
        status: "done",
        report: "结论：无问题",
        hi: 2,
        childIndex: 0,
      },
    ]);
  });

  test("a failed call reads failed, not done", () => {
    const rows = [feedRow(2, "delegate", '{"task":"探索仓库"}', "子代理超时", true)];
    expect(childrenFromFeed(rows)).toEqual([
      {
        childId: undefined,
        name: "subagent",
        task: "探索仓库",
        status: "failed",
        report: "子代理超时",
        hi: 2,
        childIndex: 0,
      },
    ]);
  });

  test("a row the mounted page also holds still folds — the merge dedupes it", () => {
    // The feed is the whole log, so it repeats what the page holds too. Its
    // `hi` is what lets `sessionChildren` recognise the page's copy.
    const rows = [
      feedRow(10, "delegate", '{"task":"探索仓库"}', "[subagent report — child_id 12 · 4 tool calls, 9s]\n结论：无问题"),
    ];
    expect(childrenFromFeed(rows)).toMatchObject([{ childId: 12, status: "done", hi: 10 }]);
  });

  test("a parallel delegation becomes one child per task", () => {
    const rows = [
      feedRow(
        3,
        "delegate",
        '{"tasks":["鉴权","性能"]}',
        "[delegate — 2 children]\n" +
          "── [0] ok · 3s ──\n[subagent report — child_id 21 · 2 tool calls, 3s]\n鉴权没问题\n" +
          "── [1] ok · 5s ──\n[subagent report — child_id 22 · 1 tool calls, 5s]\n性能可以",
      ),
    ];
    expect(childrenFromFeed(rows)).toMatchObject([
      { childId: 21, name: "subagent #1", task: "鉴权", status: "done", report: "鉴权没问题", hi: 3 },
      { childId: 22, name: "subagent #2", task: "性能", status: "done", report: "性能可以", hi: 3 },
    ]);
  });

  test("answer_child's result is the resumed child's next outcome", () => {
    const parked = feedRow(2, "delegate", '{"task":"探索仓库"}', "[subagent reviewer (child_id 7) is waiting for you]\n需要确认分支");
    const answer = (content: string, isError = false) => ({
      ...feedRow(4, "answer_child", '{"child_id":7,"answer":"用 main"}', content),
      is_error: isError,
    });
    // It reported: the card settles as done, keeping its original task.
    expect(childrenFromFeed([parked, answer("[subagent report — child_id 7 · 2 tool calls, 4s]\n已切到 main")])).toMatchObject([
      { childId: 7, task: "探索仓库", status: "done", report: "已切到 main", hi: 2 },
    ]);
    // It failed.
    expect(childrenFromFeed([parked, answer("编译失败", true)])).toMatchObject([{ childId: 7, status: "failed" }]);
    // It parked again on a new question.
    expect(
      childrenFromFeed([parked, answer("[subagent reviewer (child_id 7) is waiting for you]\n再确认一次")]),
    ).toMatchObject([{ childId: 7, status: "waiting", waiting: "再确认一次" }]);
    // It was cancelled while answering: the marker IS the result, and must read
    // as a cancel rather than as a report.
    expect(childrenFromFeed([parked, answer("[subagent cancelled — child_id 7]", true)])).toMatchObject([
      { childId: 7, status: "aborted" },
    ]);
  });
});

describe("sessionChildren", () => {
  /** A view whose mounted page starts at history 10 and holds one child. */
  const mountedView = () =>
    ({
      ...applyEvents(emptyView(), [
        { UserPrompt: "委派" },
        card(30),
        {
          ToolCallFinished: {
            name: "review #30",
            ok: true,
            content: "[subagent report — child_id 30 · 1 tool calls, 2s]\n新页结论",
            parent: "delegate",
            child_id: 30,
          },
        },
      ] as UiEvent[]),
      historyStart: 10,
      historyTotal: 20,
    });

  const earlier = () =>
    childrenFromFeed([
      feedRow(5, "delegate", '{"task":"早期任务"}', "[subagent report — child_id 12 · 4 tool calls, 9s]\n旧页结论"),
    ]);

  test("no feed: the mounted page is the whole list", () => {
    expect(sessionChildren(mountedView()).map((c) => [c.childId, c.status])).toEqual([[30, "done"]]);
  });

  test("the feed's order leads; children only the page has follow it", () => {
    const view = { ...mountedView(), feedChildren: earlier() };
    expect(sessionChildren(view).map((c) => [c.childId, c.status])).toEqual([
      [12, "done"],
      [30, "done"],
    ]);
  });

  test("a child both sources know shows once, settled", () => {
    // The feed's snapshot predates the report; the page loaded after it.
    const stale = { ...earlier()[0], childId: 30, status: "running" as const, hi: 10 };
    const view = { ...mountedView(), feedChildren: [stale] };
    expect(sessionChildren(view).map((c) => [c.childId, c.status])).toEqual([[30, "done"]]);
  });

  test("a delegate row above the page, the answer_child row inside it", () => {
    // The page holds the `answer_child` rows but no delegate card, so alone it
    // could never draw the child: the feed's settled copy is the whole answer.
    const page = {
      ...viewFromHistory(
        [
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
          {
            role: "tool",
            content: "[subagent report — child_id 7 · 2 tool calls, 4s]\n已切到 main",
            tool_call_id: "a1",
            ts: 50,
          },
        ],
        undefined,
        10,
      ),
      historyStart: 10,
      historyTotal: 14,
    };
    expect(extractChildren(page)).toEqual([]);
    const view = {
      ...page,
      feedChildren: childrenFromFeed([
        feedRow(5, "delegate", '{"task":"探索仓库"}', "[subagent reviewer (child_id 7) is waiting for you]\n需要确认分支"),
        feedRow(
          12,
          "answer_child",
          '{"child_id":7,"answer":"用 main"}',
          "[subagent report — child_id 7 · 2 tool calls, 4s]\n已切到 main",
        ),
      ]),
    };
    expect(sessionChildren(view).map((c) => [c.childId, c.status])).toEqual([[7, "done"]]);
  });

  test("a page still showing the parked card reads done once the feed settled it", () => {
    const page = {
      ...viewFromHistory(
        [
          {
            role: "assistant",
            content: "",
            ts: 20,
            tool_calls: [
              { id: "d1", type: "function", function: { name: "delegate", arguments: '{"task":"探索仓库"}' } },
            ],
          },
          {
            role: "tool",
            content: "[subagent reviewer (child_id 7) is waiting for you]\n需要确认分支",
            tool_call_id: "d1",
            ts: 30,
          },
        ],
        undefined,
        5,
      ),
      historyStart: 0,
    };
    expect(sessionChildren(page).map((c) => [c.childId, c.status])).toEqual([[7, "waiting"]]);
    const view = {
      ...page,
      feedChildren: childrenFromFeed([
        feedRow(5, "delegate", '{"task":"探索仓库"}', "[subagent reviewer (child_id 7) is waiting for you]\n需要确认分支"),
        feedRow(
          12,
          "answer_child",
          '{"child_id":7,"answer":"用 main"}',
          "[subagent report — child_id 7 · 2 tool calls, 4s]\n已切到 main",
        ),
      ]),
    };
    expect(sessionChildren(view).map((c) => [c.childId, c.status])).toEqual([[7, "done"]]);
  });

  test("a page boundary cutting between the delegate row and its card loses nothing", () => {
    const view = {
      ...emptyView(),
      historyStart: 10,
      historyTotal: 20,
      feedChildren: childrenFromFeed([
        feedRow(10, "delegate", '{"task":"探索仓库"}', "[subagent report — child_id 12 · 4 tool calls, 9s]\n结论：无问题"),
      ]),
    };
    expect(sessionChildren(view).map((c) => [c.childId, c.status])).toEqual([[12, "done"]]);
  });

  test("a live status outranks the feed's snapshot", () => {
    // Parked in the log, resumed by the live stream since.
    const page = applyEvents(emptyView(), [card(7)] as UiEvent[]);
    const view = {
      ...page,
      feedChildren: childrenFromFeed([
        feedRow(5, "delegate", '{"task":"探索仓库"}', "[subagent reviewer (child_id 7) is waiting for you]\n需要确认分支"),
      ]),
    };
    expect(sessionChildren(view).map((c) => [c.childId, c.status])).toEqual([[7, "running"]]);
  });

  test("an id-less parallel child is matched by its row, not listed twice", () => {
    // `index` is the result row's own position — the same coordinate the card's
    // `hi` carries.
    const row = feedRow(4, "delegate", '{"tasks":["鉴权"]}', "子代理完成");
    const page = viewFromHistory(
      [
        {
          role: "assistant",
          content: "",
          ts: 20,
          tool_calls: [
            { id: "d1", type: "function", function: { name: "delegate", arguments: '{"tasks":["鉴权"]}' } },
          ],
        },
        { role: "tool", content: "子代理完成", tool_call_id: "d1", ts: 30 },
      ],
      undefined,
      3,
    );
    const out = sessionChildren({ ...page, feedChildren: childrenFromFeed([row]) });
    expect(out).toMatchObject([{ childId: undefined, status: "done", hi: 4 }]);
    expect(out).toHaveLength(1);
  });
});

describe("withoutDismissed", () => {
  test("a cleared child stays gone while the session keeps streaming", () => {
    const v = applyEvents(emptyView(), [card(1), card(2)] as UiEvent[]);
    const dismissed = new Set(["c1"]);
    expect(withoutDismissed(sessionChildren(v), dismissed).map((c) => c.childId)).toEqual([2]);
    // The next event rebuilds the list from the log — the cleared card must not
    // come back with it.
    const later = applyEvents(v, [{ TextDelta: { text: "继续", parent: null } }] as UiEvent[]);
    expect(withoutDismissed(sessionChildren(later), dismissed).map((c) => c.childId)).toEqual([2]);
  });

  test("a child with no id is remembered by the row it came from, and its place there", () => {
    const feed = childrenFromFeed([feedRow(4, "delegate", '{"tasks":["鉴权"]}', "子代理完成")]);
    const view = { ...emptyView(), feedChildren: feed };
    expect(withoutDismissed(sessionChildren(view), new Set(["h4-i0"]))).toEqual([]);
  });

  test("two id-less children of one batch are cleared apart", () => {
    // Both cards come off ONE `delegate` row, so they share its `hi` — the
    // place in the batch is what keeps clearing one from clearing the other.
    const feed = childrenFromFeed([
      feedRow(
        4,
        "delegate",
        '{"tasks":["鉴权","性能"]}',
        "[delegate — 2 children]\n── [0] ok · 3s ──\n超时\n── [1] ok · 1s ──\n超时",
      ),
    ]);
    expect(feed.map((c) => [c.hi, c.childIndex])).toEqual([
      [4, 0],
      [4, 1],
    ]);
    const view = { ...emptyView(), feedChildren: feed };
    expect(withoutDismissed(sessionChildren(view), new Set([`h4-i1`]))).toHaveLength(1);
  });

  test("nothing cleared hands the same list back", () => {
    const v = applyEvents(emptyView(), [card(1)] as UiEvent[]);
    const children = sessionChildren(v);
    expect(withoutDismissed(children, new Set())).toBe(children);
  });
});
