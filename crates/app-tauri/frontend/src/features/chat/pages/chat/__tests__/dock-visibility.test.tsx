import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { ChatPage } from "../index";
import { RightSidebarToggle } from "@/components/layout-controls";
import { useDockStore } from "@/features/chat/components/dock/state";
import { clearWorkspaceChanges } from "@/features/chat/components/changes-panel/workspace-changes";
import { emptyView } from "@/features/chat/hooks/stream-view";
import { useAgentStore } from "@/stores/agent-store";
import { renderWithProviders } from "@/test/render";
import { mockCommand } from "@/test/tauri";

vi.mock("@/components/title-bar", () => ({ TitleBar: () => <RightSidebarToggle /> }));
vi.mock("@/features/chat/components/chat-stream", () => ({ ChatStream: () => null }));
vi.mock("@/features/chat/components/composer-bar", () => ({ ComposerBar: () => null }));

beforeEach(() => {
  clearWorkspaceChanges();
  useAgentStore.setState(useAgentStore.getInitialState(), true);
  useAgentStore.setState({ workspaceRoot: "/project", activeId: 1 });
  useDockStore.setState({
    open: false, tab: "changes", browserOpen: false, filesTab: "changes", width: 300,
  });
  mockCommand("agent_session", () => ({ jobs: [] }), "bg_list");
  mockCommand("agent_session", () => ({ repo: false, files: [] }), "workspace_changes");
  const view = emptyView();
  view.changes = [{
    path: "src/main.ts", kind: "update", adds: 1, dels: 1, fuzzy: false,
    patches: [{ n: 1, tool: "fuzzy_patch", diff: "@@ -1 +1 @@\n-old\n+new" }],
  }];
  useAgentStore.getState().loadView("/project", 1, view, { historyStart: 0 });
});
afterEach(() => clearWorkspaceChanges());

test.each([false, true])(
  "the first visible sidebar really collapses without changing tabs (initial open=%s)",
  async (initialOpen) => {
    useDockStore.setState({ open: initialOpen });
    renderWithProviders(
      <ChatPage title="会话" workspaceRoot="/project" sessionKey="/project:1" sessionId={1} />,
    );
    await waitFor(() => expect(useDockStore.getState().open).toBe(true));
    const selected = useDockStore.getState().tab;
    fireEvent.click(screen.getByRole("button", { name: "收起工具侧栏" }));
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 50)); });
    expect(useDockStore.getState().open).toBe(false);
    expect(useDockStore.getState().tab).toBe(selected);
    expect(screen.getByRole("button", { name: "展开工具侧栏" })).toBeVisible();
    const frame = document.getElementById("workspace-tools-sidebar")?.parentElement;
    expect(frame).toHaveStyle({ width: "0px" });
    expect(frame).toHaveAttribute("aria-hidden", "true");
  },
);

test("working mode does not fetch git changes or resurrect coding-mode tabs", async () => {
  const git = vi.fn(() => ({ repo: true, files: [] }));
  mockCommand("agent_session", git, "workspace_changes");
  useDockStore.setState({ open: true, tab: "files", browserOpen: true });
  renderWithProviders(
    <ChatPage title="工作" workspaceRoot="/project" officeMode sessionKey="/project:1" sessionId={1} />,
  );
  await act(async () => { await Promise.resolve(); });
  expect(screen.queryByRole("tab", { name: /^文件/ })).toBeNull();
  expect(screen.queryByRole("tab", { name: /^改动/ })).toBeNull();
  expect(screen.queryByRole("tab", { name: "工具" })).toBeNull();
  expect(screen.getByText("工作区工具")).toBeVisible();
  expect(git).not.toHaveBeenCalled();
});
