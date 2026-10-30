import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { useWorkspaceChanges } from "../use-workspace-changes";
import { emptyView, type SessionView } from "../stream-view";
import { clearWorkspaceChanges, refreshWorkspaceChanges } from "@/features/chat/components/changes-panel/workspace-changes";
import { useAgentStore } from "@/stores/agent-store";
import { deferred, mockCommand } from "@/test/tauri";
import type { WorkspaceChangeRow } from "@/lib/agent-ipc";

const Probe = ({ root }: { root: string }) => {
  const snapshot = useWorkspaceChanges(root);
  return <output data-testid="workspace-changes">{JSON.stringify(snapshot)}</output>;
};

/** Install a view for a session of the workspace — the event pump's effect. */
function putView(key: string, view: SessionView) {
  const views = new Map(useAgentStore.getState().views);
  views.set(key, view);
  useAgentStore.setState({ views });
}

const ok = vi.fn(() => Promise.resolve({ repo: true, files: [] }));

beforeEach(() => {
  clearWorkspaceChanges();
  useAgentStore.setState(useAgentStore.getInitialState(), true);
  useAgentStore.setState({ workspaceRoot: "/project", activeId: 1 });
  ok.mockClear();
  mockCommand("agent_session", ok, "workspace_changes");
});
afterEach(() => clearWorkspaceChanges());

describe("the 改动 panel's triggers", () => {
  test("switching projects never shows the old snapshot or a late old error", async () => {
    const late = deferred<{ repo: boolean; files: WorkspaceChangeRow[] }>();
    let oldReads = 0;
    mockCommand("agent_session", (args: Record<string, unknown> | undefined) => {
      if (args?.path === "/other") return Promise.reject("other project failed");
      if (oldReads++ > 0) return late.promise;
      return {
        repo: true,
        files: [{
          path: "old-project.ts", kind: "update", adds: 1, dels: 0,
          diff: "", binary: false, truncated: false,
        }],
      };
    }, "workspace_changes");
    const { rerender } = render(<Probe root="/project" />);
    await act(async () => { await Promise.resolve(); });
    expect(screen.getByTestId("workspace-changes").textContent).toContain("old-project.ts");
    let pending!: Promise<void>;
    act(() => { pending = refreshWorkspaceChanges("/project"); });
    rerender(<Probe root="/other" />);
    expect(screen.getByTestId("workspace-changes").textContent).not.toContain("old-project.ts");
    await act(async () => { await Promise.resolve(); });
    const snapshot = () => JSON.parse(screen.getByTestId("workspace-changes").textContent!);
    expect(snapshot()).toMatchObject({ repo: null, at: 0, files: [], error: "other project failed" });
    await act(async () => {
      late.reject("late old-project failure");
      await pending;
    });
    expect(snapshot()).toMatchObject({ repo: null, at: 0, files: [], error: "other project failed" });
  });

  test("a late successful scan only populates its own project", async () => {
    const late = deferred<{ repo: boolean; files: WorkspaceChangeRow[] }>();
    mockCommand("agent_session", (args: Record<string, unknown> | undefined) =>
      args?.path === "/project" ? late.promise : { repo: true, files: [] },
    "workspace_changes");
    const { rerender } = render(<Probe root="/project" />);
    rerender(<Probe root="/other" />);
    await act(async () => {
      late.resolve({
        repo: true,
        files: [{
          path: "late-old-project.ts", kind: "add", adds: 1, dels: 0,
          diff: "", binary: false, truncated: false,
        }],
      });
      await Promise.resolve();
    });
    expect(screen.getByTestId("workspace-changes").textContent).not.toContain("late-old-project.ts");
    expect(JSON.parse(screen.getByTestId("workspace-changes").textContent!))
      .toMatchObject({ repo: true, files: [], error: null });
  });

  test("mounting and a window that comes back both re-read", async () => {
    render(<Probe root="/project" />);
    await act(async () => { await Promise.resolve(); });
    expect(ok).toHaveBeenCalledTimes(1);
    // A `git commit` in a terminal, or an editor save, leaves no event of
    // ours — the window coming back is the cue.
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    await act(async () => { await Promise.resolve(); });
    expect(ok).toHaveBeenCalledTimes(2);
  });

  test("a finished write in ANY session of the workspace re-reads, debounced", async () => {
    vi.useFakeTimers();
    render(<Probe root="/project" />);
    await act(async () => { await vi.advanceTimersByTimeAsync(0); });
    expect(ok).toHaveBeenCalledTimes(1);

    const view = emptyView();
    view.items = [{ kind: "tool", name: "apply_patch", args: "", ts: 1 }];
    // Not the foreground session: a turn in a parked one dirties the same tree.
    act(() => putView("/project:7", view));
    // A burst of writes earns one read once it settles.
    expect(ok).toHaveBeenCalledTimes(1);
    await act(async () => { await vi.advanceTimersByTimeAsync(300); });
    expect(ok).toHaveBeenCalledTimes(2);
  });

  test("text deltas inside a running turn do not re-read on every frame", async () => {
    vi.useFakeTimers();
    render(<Probe root="/project" />);
    await act(async () => { await vi.advanceTimersByTimeAsync(0); });
    expect(ok).toHaveBeenCalledTimes(1);

    const view = emptyView();
    view.streaming = true;
    view.items = [{ kind: "assistant", text: "写", streaming: true }];
    act(() => putView("/project:1", view));
    await act(async () => { await vi.advanceTimersByTimeAsync(300); });
    // The turn's own edges are worth a read; the deltas in between are not.
    expect(ok).toHaveBeenCalledTimes(2);
    for (const text of ["写到", "写到一", "写到一半"]) {
      act(() =>
        putView("/project:1", { ...view, items: [{ kind: "assistant", text, streaming: true }] }),
      );
    }
    await act(async () => { await vi.advanceTimersByTimeAsync(300); });
    expect(ok).toHaveBeenCalledTimes(2);
  });
});
