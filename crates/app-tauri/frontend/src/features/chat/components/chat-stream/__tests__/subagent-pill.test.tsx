// The subagents capsule: how many children are out, and what each was handed.

import type { ComponentProps } from "react";
import { fireEvent, screen } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";
import { ChatStream } from "@/features/chat/components/chat-stream";
import { emptyView } from "@/features/chat/hooks/stream-view";
import { renderWithProviders } from "@/test/render";

vi.mock("@/features/chat/components/chat-stream/markdown-stream", () => ({
  MemoStreamdown: ({ text }: { text: string }) => <div>{text}</div>,
}));

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

  test("lists each child's task when opened", () => {
    renderStream({ subagents: [child(1, "review #1", "检查鉴权分支")] });
    fireEvent.click(screen.getByRole("button", { name: "子代理" }));
    expect(screen.getByText("review #1")).toBeInTheDocument();
    expect(screen.getByText("检查鉴权分支")).toBeInTheDocument();
  });

  test("yields to the pill ahead of it", () => {
    renderStream({
      subagents: [child(1, "review #1", "检查鉴权")],
      jobs: { text: "1 个任务运行中", tone: "running" },
      onToggleJobs: () => {},
    });
    expect(screen.getByRole("button", { name: "子代理" }).getAttribute("data-compact")).toBe("true");
  });
});
