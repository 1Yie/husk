import { act, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { useWorkspaceChanges } from "../use-workspace-changes";
import { emptyView, type SessionView } from "../stream-view";
import { clearWorkspaceChanges } from "@/features/chat/components/changes-panel/workspace-changes";
import { useAgentStore } from "@/stores/agent-store";
import { mockCommand } from "@/test/tauri";

const Probe = ({ root }: { root: string }) => {
  useWorkspaceChanges(root);
  return null;
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
