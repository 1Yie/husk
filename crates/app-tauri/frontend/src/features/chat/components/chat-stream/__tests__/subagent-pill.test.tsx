// The subagents capsule: how many children are out, and that it opens the
// dock's 子代理 view.

import type { ComponentProps } from "react";
import { fireEvent, screen } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";
import { ChatStream } from "@/features/chat/components/chat-stream";
import { emptyView } from "@/features/chat/hooks/stream-view";
import { renderWithProviders } from "@/test/render";

vi.mock("@/features/chat/components/chat-stream/markdown-stream", () => ({
  MemoStreamdown: ({ text }: { text: string }) => <div>{text}</div>,
}));
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

const child = (childId: number, name: string, task: string) => ({ childId, name, task });

function renderStream(props: Partial<ComponentProps<typeof ChatStream>> = {}) {
  return renderWithProviders(<ChatStream view={emptyView()} sessionKey="/project:1" {...props} />);
}

describe("subagent capsule", () => {
  test("stays off while no child runs", () => {
    renderStream({ subagents: [] });
    expect(screen.queryByRole("button", { name: "子代理" })).toBeNull();
  });

  test("counts the running children", () => {
    renderStream({
      subagents: [child(1, "review #1", "检查鉴权"), child(2, "review #2", "看性能")],
    });
    const pill = screen.getByRole("button", { name: "子代理" });
    expect(pill.textContent).toContain("2 个子代理运行中");
    expect(pill.getAttribute("data-compact")).toBeNull();
  });

  test("clicking it opens the 子代理 view instead of a popover", () => {
    const onToggleSubagents = vi.fn();
    renderStream({
      subagents: [child(1, "review #1", "检查鉴权分支")],
      onToggleSubagents,
    });
    fireEvent.click(screen.getByRole("button", { name: "子代理" }));
    expect(onToggleSubagents).toHaveBeenCalledTimes(1);
    // The rows live in the sidebar now, never in a popover over the composer.
    expect(screen.queryByText("检查鉴权分支")).not.toBeInTheDocument();
  });

  test("reads as pressed while the dock shows the 子代理 view", () => {
    renderStream({
      subagents: [child(1, "review #1", "检查鉴权")],
      subagentsOpen: true,
      onToggleSubagents: () => {},
    });
    const pill = screen.getByRole("button", { name: "子代理" });
    expect(pill.getAttribute("aria-pressed")).toBe("true");
  });

  test("yields to the pill ahead of it", () => {
    renderStream({
      subagents: [child(1, "review #1", "检查鉴权")],
      jobs: { text: "1 个任务运行中", tone: "running" },
      onToggleJobs: () => {},
    });
    expect(screen.getByRole("button", { name: "子代理" }).getAttribute("data-compact")).toBe("true");
  });

  test("a replayed transcript nests under its child's card", () => {
    // Exactly the shape `applyChildTranscript` installs: the card with its body
    // segments, then the child's own tool rows right behind it.
    const view = emptyView();
    view.items = [
      { kind: "user", text: "委派子代理" },
      { kind: "tool", name: "delegate", args: "1 task", content: "父级汇总", ok: true },
      {
        kind: "tool",
        name: "review #1",
        args: "看鉴权",
        content: "报告",
        ok: true,
        parent: "delegate",
        childId: 7,
        segments: [{ kind: "text", text: "找到三处" }, { kind: "tool", index: 0 }],
      },
      { kind: "tool", name: "smart_grep", args: "auth", content: "3 hits", ok: true, parent: "review #1", childId: 7 },
    ];
    renderWithProviders(<ChatStream view={view} sessionKey="/project:1" />);
    // The tools phase is collapsed until asked — the chip list is its body.
    fireEvent.click(screen.getByRole("button", { name: /次工具调用/ }));
    expect(screen.getByText("找到三处")).toBeInTheDocument();
    expect(screen.getByText("smart_grep")).toBeInTheDocument();
  });
});
