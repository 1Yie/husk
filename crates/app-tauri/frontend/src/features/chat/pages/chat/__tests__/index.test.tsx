import { act } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { ChatPage } from "../index";
import { activeViewNow, initAgentStore, useAgentStore } from "@/stores/agent-store";
import { emptyView, type BgJob, type FileChange } from "@/features/chat/hooks/stream-view";
import { clearWorkspaceChanges } from "@/features/chat/components/changes-panel/workspace-changes";
import { renderWithProviders } from "@/test/render";
import { deferred, emitAgentEvent, mockCommand } from "@/test/tauri";

// The seed is the page's own job; the panes it hosts have their own tests.
vi.mock("@/components/title-bar", () => ({ TitleBar: () => null }));
vi.mock("@/features/chat/components/chat-stream", () => ({ ChatStream: () => null }));
vi.mock("@/features/chat/components/composer-bar", () => ({ ComposerBar: () => null }));
// The dock is a dumb frame: these tests read the `changes` it is handed.
const { dockProps } = vi.hoisted(() => ({ dockProps: vi.fn() }));
vi.mock("@/features/chat/components/dock", () => ({
  SessionDock: (props: unknown) => {
    dockProps(props);
    return null;
  },
}));

const job = (id: string): BgJob => ({
  id,
  command: "cargo test",
  state: { kind: "running" },
  started_at_ms: 1,
  elapsed_ms: 1000,
  notify: true,
  has_own_sink: false,
});

let dispose: (() => void) | undefined;
beforeEach(() => {
  vi.useFakeTimers();
  useAgentStore.setState(useAgentStore.getInitialState(), true);
  useAgentStore.setState({ workspaceRoot: "/project", activeId: 1 });
});
afterEach(() => {
  dispose?.();
  dispose = undefined;
  clearWorkspaceChanges();
});

function renderPage() {
  return renderWithProviders(
    <ChatPage title="会话" workspaceRoot="/project" sessionKey="/project:1" sessionId={1} />,
  );
}
const seeded = () => activeViewNow().bgJobs.map((j) => j.id);
const dockChanges = (): FileChange[] =>
  (dockProps.mock.lastCall?.[0] as { changes: FileChange[] } | undefined)?.changes ?? [];

describe("the 后台 seed", () => {
  test("a read that starts before the view exists still lands", async () => {
    const seed = deferred<{ jobs: BgJob[] }>();
    const bgList = vi.fn(() => seed.promise);
    mockCommand("agent_session", bgList, "bg_list");
    renderPage();
    await act(async () => { await Promise.resolve(); });
    // The session pointer flips before its history is folded, so there is
    // nothing to write into yet — the read waits instead of landing on the floor.
    expect(bgList).toHaveBeenCalledTimes(0);
    act(() => useAgentStore.getState().loadView("/project", 1, emptyView(), { historyStart: 0 }));
    await act(async () => { await Promise.resolve(); });
    expect(bgList).toHaveBeenCalledTimes(1);
    seed.resolve({ jobs: [job("bg-1")] });
    await act(async () => { await Promise.resolve(); });
    expect(seeded()).toEqual(["bg-1"]);
  });

  test("a live snapshot is not overwritten by a seed that resolves later", async () => {
    dispose = initAgentStore();
    const seed = deferred<{ jobs: BgJob[] }>();
    mockCommand("agent_session", () => seed.promise, "bg_list");
    renderPage();
    act(() => useAgentStore.getState().loadView("/project", 1, emptyView(), { historyStart: 0 }));
    await act(async () => {
      emitAgentEvent({
        root: "/project",
        session: 1,
        event: { Custom: { kind: "husk:bg_jobs", payload: { jobs: [job("bg-2")] } } },
      });
      await vi.advanceTimersByTimeAsync(16);
    });
    seed.resolve({ jobs: [job("bg-1")] });
    await act(async () => { await Promise.resolve(); });
    expect(seeded()).toEqual(["bg-2"]);
  });
});

describe("改动 comes from the workspace, not the session log", () => {
  const sessionChange = (path: string): FileChange => ({
    path,
    kind: "update",
    patches: [{ n: 1, tool: "fuzzy_patch", diff: "@@ -1 +1 @@\n-old\n+new" }],
    adds: 1,
    dels: 1,
    fuzzy: false,
  });
  const loadSessionChange = (path: string) => {
    const view = emptyView();
    view.changes = [sessionChange(path)];
    act(() => useAgentStore.getState().loadView("/project", 1, view, { historyStart: 0 }));
  };
  const settle = () => act(async () => { await vi.advanceTimersByTimeAsync(0); });

  test("a repository's uncommitted files are what the dock shows", async () => {
    mockCommand(
      "agent_session",
      () => ({
        repo: true,
        files: [{ path: "src/w.ts", kind: "update", adds: 3, dels: 0, diff: "@@ -1 +1,2 @@\n-a\n+b\n+c", binary: false, truncated: false }],
      }),
      "workspace_changes",
    );
    // The session wrote one file of its own — replayed history would list only
    // that, while the checkout is what the panel promises.
    loadSessionChange("src/session-only.ts");
    renderPage();
    await settle();
    expect(dockChanges().map((c) => c.path)).toEqual(["src/w.ts"]);
    expect(dockChanges()[0].patches).toMatchObject([{ tool: "git", n: 1 }]);
  });

  test("a plain folder falls back to the session's own list", async () => {
    loadSessionChange("src/session-only.ts");
    renderPage();
    await settle();
    expect(dockChanges().map((c) => c.path)).toEqual(["src/session-only.ts"]);
  });
});
