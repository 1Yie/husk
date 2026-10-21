// The jobs pill: the session's background jobs in one line beside the dock
// pill. It exists only while the session has jobs.

import type { ComponentProps } from "react";
import { fireEvent, screen } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";
import { ChatStream } from "@/features/chat/components/chat-stream";
import { emptyView } from "@/features/chat/hooks/stream-view";
import { renderWithProviders } from "@/test/render";

vi.mock("@/features/chat/components/chat-stream/markdown-stream", () => ({
  MemoStreamdown: ({ text }: { text: string }) => <div>{text}</div>,
}));

function renderStream(props: Partial<ComponentProps<typeof ChatStream>> = {}) {
  return renderWithProviders(
    <ChatStream view={emptyView()} sessionKey="/project:1" onToggleJobs={() => {}} {...props} />,
  );
}

describe("jobs pill", () => {
  test("stays off while the session has no jobs", () => {
    renderStream({ jobs: null });
    expect(screen.queryByRole("button", { name: "后台任务" })).toBeNull();
  });

  test("says what the jobs are doing, with the tone on the button", () => {
    renderStream({ jobs: { text: "2 个任务运行中", tone: "running" } });
    const pill = screen.getByRole("button", { name: "后台任务" });
    expect(pill.textContent).toContain("2 个任务运行中");
    expect(pill.getAttribute("data-jobs-tone")).toBe("running");
  });

  test("sits beside the dock pill, each with its own toggle", () => {
    const onToggleJobs = vi.fn();
    const onToggleDock = vi.fn();
    renderStream({
      jobs: { text: "1 个任务失败", tone: "failed" },
      onToggleJobs,
      dockCount: 1,
      dockTab: "changes",
      onToggleDock,
    });
    fireEvent.click(screen.getByRole("button", { name: "后台任务" }));
    expect(onToggleJobs).toHaveBeenCalledTimes(1);
    expect(onToggleDock).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "侧栏" }));
    expect(onToggleDock).toHaveBeenCalledTimes(1);
  });

  test("reads as pressed while the dock shows 后台", () => {
    renderStream({ jobs: { text: "1 个任务已结束", tone: "ok" }, jobsOpen: true });
    expect(screen.getByRole("button", { name: "后台任务" }).getAttribute("aria-pressed")).toBe("true");
  });
});
