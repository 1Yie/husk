// The dock pill speaks for THIS session: it appears for the files the session
// produced, never for the repository's own dirty state.

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
    <ChatStream view={emptyView()} sessionKey="/project:1" onToggleDock={() => {}} {...props} />,
  );
}

describe("dock pill", () => {
  test("stays off while the session has produced nothing", () => {
    renderStream({ dockCount: 0 });
    expect(screen.queryByRole("button", { name: "侧栏" })).toBeNull();
  });

  test("shows up for the session's changes with the diff's own numbers", () => {
    renderStream({ dockCount: 2, dockTab: "changes", dockAdds: 12, dockDels: 4 });
    const pill = screen.getByRole("button", { name: "侧栏" }).textContent ?? "";
    expect(pill).toContain("+12");
    expect(pill).toContain("−4");
  });

  test("names the file count when the active view is 产物", () => {
    renderStream({ dockCount: 3, dockTab: "artifacts" });
    expect(screen.getByRole("button", { name: "侧栏" }).textContent).toContain("3 个文件");
  });

  test("pressing it asks the page to toggle the sidebar", () => {
    const onToggleDock = vi.fn();
    renderStream({ dockCount: 1, dockTab: "changes", onToggleDock });
    fireEvent.click(screen.getByRole("button", { name: "侧栏" }));
    expect(onToggleDock).toHaveBeenCalledTimes(1);
  });
});
