// A batch delegation restored from history draws one card per child, and every
// one of those cards comes off the SAME `delegate` row — so they all carry that
// row's `hi`. The row id a card's open state is keyed by (and its React key)
// therefore has to name the child as well, or expanding one card expanded every
// sibling and the keys collided.

import { fireEvent, screen } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";
import { ChatStream } from "@/features/chat/components/chat-stream";
import { extractChildren } from "@/features/chat/hooks/running-children";
import { toolRowId, type SessionView } from "@/features/chat/hooks/stream-view";
import { viewFromHistory } from "@/features/chat/hooks/view-from-history";
import type { ChatMessage } from "@/types";
import { renderWithProviders } from "@/test/render";

// Markdown is a separate surface; the card's own text is what these tests read.
vi.mock("@/features/chat/components/chat-stream/markdown-stream", () => ({
  MemoStreamdown: ({ text }: { text: string }) => <div>{text}</div>,
}));
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

/** A reopened session whose turn delegated two children in parallel — the
 *  persisted result carries one `── [i] ok · Ns ──` section per child. */
function batchHistory(reports: [string, string]): ChatMessage[] {
  return [
    { role: "user", content: "并行委派", ts: 10 },
    {
      role: "assistant",
      content: "",
      ts: 20,
      tool_calls: [
        {
          id: "d1",
          type: "function",
          function: { name: "delegate", arguments: '{"tasks":["看鉴权","看性能"]}' },
        },
      ],
    },
    {
      role: "tool",
      content:
        "[delegate — 2 children]\n" +
        `── [0] ok · 3s ──\n${reports[0]}\n` +
        `── [1] ok · 5s ──\n${reports[1]}`,
      tool_call_id: "d1",
      ts: 30,
    },
  ];
}

/** The folded child cards, each with the index the stream folds it at. */
function childCards(view: SessionView) {
  return view.items.flatMap((item, idx) =>
    item.kind === "tool" && item.parent === "delegate" ? [{ card: item, idx }] : [],
  );
}

const namedReports: [string, string] = [
  "[subagent report — child_id 21 · 2 tool calls, 3s]\n鉴权没问题",
  "[subagent report — child_id 22 · 1 tool calls, 5s]\n性能可以",
];

describe("a batch delegation restored from history", () => {
  test("each child card carries its own row id, not the batch's", () => {
    const view = viewFromHistory(batchHistory(namedReports));
    const kids = childCards(view);
    expect(kids.map(({ card }) => card.childId)).toEqual([21, 22]);
    // All of them share the parent row's `hi` — that is the coordinate that
    // used to collapse their ids into one.
    expect(new Set(kids.map(({ card }) => card.hi)).size).toBe(1);
    const ids = kids.map(({ card, idx }) => toolRowId(card, idx));
    expect(new Set(ids).size).toBe(2);
    // Derivable from the parent row each time, so a prepended page cannot
    // renumber it — and never the live path's bare `tool-<idx>`.
    expect(ids).toEqual([`tool-${kids[0].card.hi}-c21`, `tool-${kids[0].card.hi}-c22`]);
  });

  test("the reports lost their child_id — the batch position still names each card", () => {
    const view = viewFromHistory(batchHistory(["鉴权没问题", "性能可以"]));
    const kids = childCards(view);
    expect(kids.map(({ card }) => card.childId)).toEqual([undefined, undefined]);
    expect(kids.map(({ card }) => card.childIndex)).toEqual([0, 1]);
    const ids = kids.map(({ card, idx }) => toolRowId(card, idx));
    expect(new Set(ids).size).toBe(2);
    expect(ids).toEqual([`tool-${kids[0].card.hi}-i0`, `tool-${kids[0].card.hi}-i1`]);
  });

  test("expanding one card opens that card alone", () => {
    renderWithProviders(<ChatStream view={viewFromHistory(batchHistory(namedReports))} sessionKey="/project:1" />);
    // The tools phase and every child card start closed.
    fireEvent.click(screen.getByRole("button", { name: /次工具调用/ }));
    const first = screen.getByRole("button", { name: /subagent #1/ });
    const second = screen.getByRole("button", { name: /subagent #2/ });
    expect(first).toHaveAttribute("aria-expanded", "false");
    expect(second).toHaveAttribute("aria-expanded", "false");

    fireEvent.click(first);

    expect(first).toHaveAttribute("aria-expanded", "true");
    expect(second).toHaveAttribute("aria-expanded", "false");
    // The sibling's body is not even mounted, so only the opened card shows a
    // report — with the old shared id both cards rendered it.
    const opened = first.parentElement?.textContent ?? "";
    const closed = second.parentElement?.textContent ?? "";
    expect(opened).toContain("鉴权没问题");
    expect(closed).not.toContain("性能可以");
  });
});

/** A reopened session whose turn ran two calls through one `batch_execute` —
 *  the persisted result carries one `── [i] tool (status) ──` section per item,
 *  and `batchChildren` folds a nested chip out of each. */
function batchExecuteHistory(): ChatMessage[] {
  return [
    { role: "user", content: "并行看两处", ts: 10 },
    {
      role: "assistant",
      content: "",
      ts: 20,
      tool_calls: [
        {
          id: "b1",
          type: "function",
          function: {
            name: "batch_execute",
            arguments: JSON.stringify({
              calls: [
                { tool: "smart_read", args: { path: "a.ts" } },
                { tool: "smart_grep", args: { pattern: "TODO" } },
              ],
            }),
          },
        },
      ],
    },
    {
      role: "tool",
      content:
        "[batch_execute — 2 calls]\n" +
        "── [0] smart_read (ok) ──\n鉴权走的是 JWT\n" +
        "── [1] smart_grep (ok) ──\nTODO 只剩两处",
      tool_call_id: "b1",
      ts: 30,
    },
  ];
}

/** The folded batch items, each with the index the stream folds it at. */
function batchItemCards(view: SessionView) {
  return view.items.flatMap((item, idx) =>
    item.kind === "tool" && item.parent === "batch_execute" ? [{ card: item, idx }] : [],
  );
}

describe("a batch_execute restored from history", () => {
  test("each nested item carries its own row id, not the capsule's", () => {
    const view = viewFromHistory(batchExecuteHistory());
    const kids = batchItemCards(view);
    expect(kids.map(({ card }) => card.name)).toEqual(["smart_read", "smart_grep"]);
    // Both ride the capsule's `hi` — the coordinate that used to collapse
    // their ids into one.
    expect(new Set(kids.map(({ card }) => card.hi)).size).toBe(1);
    expect(kids.map(({ card }) => card.childIndex)).toEqual([0, 1]);
    const ids = kids.map(({ card, idx }) => toolRowId(card, idx));
    expect(new Set(ids).size).toBe(2);
    // Same shape a child of a `delegate` batch gets: the parent's `hi` plus its
    // own half, never the live path's bare `tool-<idx>`.
    expect(ids).toEqual([`tool-${kids[0].card.hi}-i0`, `tool-${kids[0].card.hi}-i1`]);
  });

  test("batch items are not subagents — the 子代理 list stays empty", () => {
    expect(extractChildren(viewFromHistory(batchExecuteHistory()))).toEqual([]);
  });

  test("expanding one item opens that item alone", () => {
    renderWithProviders(
      <ChatStream view={viewFromHistory(batchExecuteHistory())} sessionKey="/project:1" />,
    );
    // The tools phase starts closed; the capsule's items live in its body.
    fireEvent.click(screen.getByRole("button", { name: /次工具调用/ }));
    const first = screen.getByRole("button", { name: /smart_read/ });
    const second = screen.getByRole("button", { name: /smart_grep/ });
    expect(first).toHaveAttribute("aria-expanded", "false");
    expect(second).toHaveAttribute("aria-expanded", "false");

    fireEvent.click(first);

    expect(first).toHaveAttribute("aria-expanded", "true");
    expect(second).toHaveAttribute("aria-expanded", "false");
    // The sibling's body is never mounted, so only the opened item shows its
    // result — with the old shared id both items rendered it.
    const opened = first.parentElement?.textContent ?? "";
    const closed = second.parentElement?.textContent ?? "";
    expect(opened).toContain("鉴权走的是 JWT");
    expect(closed).not.toContain("TODO 只剩两处");
  });
});
