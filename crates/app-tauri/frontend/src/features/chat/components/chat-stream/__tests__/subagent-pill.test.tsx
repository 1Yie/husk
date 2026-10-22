// The subagents capsule: how many children are out, and what each was handed.

import type { ComponentProps } from "react";
import { fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, test, vi } from "vitest";
import { toast } from "sonner";
import { ChatStream } from "@/features/chat/components/chat-stream";
import { emptyView } from "@/features/chat/hooks/stream-view";
import { renderWithProviders } from "@/test/render";
import { mockCommand, tauri } from "@/test/tauri";
import { WAITING_STEER_HINT } from "@/lib/subagents";

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

  test("a steer typed against a child reaches the kernel", async () => {
    const user = userEvent.setup();
    renderStream({ subagents: [child(3, "review #1", "检查鉴权")] });
    await user.click(screen.getByRole("button", { name: "子代理" }));
    const box = screen.getByPlaceholderText("插话…");
    await user.type(box, "先看中间件{Enter}");
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", {
      cmd: { SteerChild: { child_id: 3, text: "先看中间件" } },
    });
    // The kernel echoes the steer back as a `subagent:steer` custom event, so
    // the box empties and nothing is drawn locally.
    await waitFor(() => expect(box).toHaveValue(""));
  });

  test("a rejected steer is reported and the draft kept", async () => {
    mockCommand("agent_cmd", () => Promise.reject(new Error("session gone")));
    const user = userEvent.setup();
    renderStream({ subagents: [child(3, "review #1", "检查鉴权")] });
    await user.click(screen.getByRole("button", { name: "子代理" }));
    const box = screen.getByPlaceholderText("插话…");
    await user.type(box, "先看中间件{Enter}");
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("插话失败：Error: session gone"));
    expect(box).toHaveValue("先看中间件");
  });

  test("a parked child says its steer waits for the resume", async () => {
    const user = userEvent.setup();
    renderStream({ subagents: [{ ...child(4, "review #1", "检查鉴权"), waiting: "用哪条分支？" }] });
    await user.click(screen.getByRole("button", { name: "子代理" }));
    expect(screen.getByText(WAITING_STEER_HINT)).toBeInTheDocument();
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
