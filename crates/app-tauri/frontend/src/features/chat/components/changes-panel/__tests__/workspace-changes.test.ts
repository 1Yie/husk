import { beforeEach, describe, expect, test, vi } from "vitest";
import { deferred, mockCommand } from "@/test/tauri";
import type { WorkspaceChangeRow } from "@/lib/agent-ipc";
import {
  clearWorkspaceChanges,
  refreshWorkspaceChanges,
  scheduleWorkspaceChanges,
  workspaceChangesNow,
} from "../workspace-changes";

const row = (path: string, over: Partial<WorkspaceChangeRow> = {}): WorkspaceChangeRow => ({
  path,
  kind: "update",
  adds: 1,
  dels: 1,
  diff: "@@ -1 +1 @@\n-old\n+new",
  binary: false,
  truncated: false,
  ...over,
});

const answer = (files: WorkspaceChangeRow[], repo = true) => ({ repo, files });

beforeEach(() => {
  clearWorkspaceChanges();
});

describe("workspace 改动", () => {
  test("maps a git row onto the panel's file shape", async () => {
    mockCommand(
      "agent_session",
      () => answer([row("src/a.ts", { adds: 2, dels: 1 }), row("logo.png", { kind: "add", adds: 0, dels: 0, diff: "", binary: true })]),
      "workspace_changes",
    );
    await refreshWorkspaceChanges("/project");
    expect(workspaceChangesNow("/project")).toMatchObject({ repo: true, loading: false });
    expect(workspaceChangesNow("/project").at).toBeGreaterThan(0);
    expect(workspaceChangesNow("/project").files).toMatchObject([
      {
        path: "src/a.ts",
        kind: "update",
        adds: 2,
        dels: 1,
        fuzzy: false,
        patches: [{ tool: "git", n: 1 }],
      },
      // No diff to stack — one patch per write is a session notion, and git
      // has nothing to draw for a binary file.
      { path: "logo.png", kind: "add", binary: true, patches: [] },
    ]);
  });

  test("one read in flight per root; an ask made meanwhile runs once after it", async () => {
    const first = deferred<{ repo: boolean; files: WorkspaceChangeRow[] }>();
    const read = vi.fn()
      .mockImplementationOnce(() => first.promise)
      .mockImplementationOnce(() => Promise.resolve(answer([row("b.ts")])));
    mockCommand("agent_session", read, "workspace_changes");

    const run = refreshWorkspaceChanges("/project");
    // Two more asks while the first is still open — they do not stack reads.
    const again = refreshWorkspaceChanges("/project");
    const third = refreshWorkspaceChanges("/project");
    expect(read).toHaveBeenCalledTimes(1);

    first.resolve(answer([row("a.ts")]));
    await run;
    await again;
    await third;
    // One re-run covers both late asks, and the list is the newest answer.
    expect(read).toHaveBeenCalledTimes(2);
    expect(workspaceChangesNow("/project").files.map((f) => f.path)).toEqual(["b.ts"]);
  });

  test("a root outside a repository says so instead of faking an empty tree", async () => {
    mockCommand("agent_session", () => answer([], false), "workspace_changes");
    await refreshWorkspaceChanges("/plain-folder");
    expect(workspaceChangesNow("/plain-folder")).toMatchObject({ repo: false, files: [] });
  });

  test("a failed read keeps the list it already had", async () => {
    mockCommand("agent_session", () => answer([row("a.ts")]), "workspace_changes");
    await refreshWorkspaceChanges("/project");
    mockCommand("agent_session", () => Promise.reject(new Error("ipc down")), "workspace_changes");
    await refreshWorkspaceChanges("/project");
    expect(workspaceChangesNow("/project")).toMatchObject({ loading: false });
    expect(workspaceChangesNow("/project").files.map((f) => f.path)).toEqual(["a.ts"]);
  });

  test("a burst of asks earns one read once the debounce closes", async () => {
    vi.useFakeTimers();
    const read = vi.fn(() => Promise.resolve(answer([row("a.ts")])));
    mockCommand("agent_session", read, "workspace_changes");
    scheduleWorkspaceChanges("/project");
    scheduleWorkspaceChanges("/project");
    scheduleWorkspaceChanges("/project");
    expect(read).toHaveBeenCalledTimes(0);
    await vi.advanceTimersByTimeAsync(300);
    expect(read).toHaveBeenCalledTimes(1);
  });
});
