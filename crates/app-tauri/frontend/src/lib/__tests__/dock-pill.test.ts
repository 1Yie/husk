// The 改动 pill follows the worktree: committed work is not announced.

import { describe, expect, test } from "vitest";
import { changesStillLive, dockPill } from "../dock-pill";

describe("changes still live", () => {
  test("a repository with an uncommitted file keeps the pill", () => {
    expect(changesStillLive({ repo: true, dirty: 1 }, 3)).toBe(true);
  });

  test("a clean repository does not — the work is committed", () => {
    expect(changesStillLive({ repo: true, dirty: 0 }, 3)).toBe(false);
  });

  test("a plain folder has nothing to commit, so its own list speaks", () => {
    expect(changesStillLive({ repo: false, dirty: 0 }, 2)).toBe(true);
    expect(changesStillLive({ repo: false, dirty: 0 }, 0)).toBe(false);
  });

  test("before the first read lands the session's list still speaks", () => {
    expect(changesStillLive(null, 2)).toBe(true);
    expect(changesStillLive(null, 0)).toBe(false);
  });
});

describe("dock pill", () => {
  const base = { filesTab: "changes" as const, changes: 3, artifacts: 0 };

  test("no files, no pill", () => {
    expect(dockPill({ ...base, changes: 0 })).toEqual({ tab: null, count: 0 });
  });

  test("the stored file view wins while it has content", () => {
    expect(dockPill({ ...base, artifacts: 2, filesTab: "artifacts" })).toEqual({
      tab: "artifacts",
      count: 2,
    });
    // Both lists have content and the reader last stood on 改动.
    expect(dockPill({ ...base, artifacts: 2 })).toEqual({ tab: "changes", count: 3 });
  });

  test("a committed change keeps its pill — the worktree only changes the numbers", () => {
    // Visibility is the session's; `changesStillLive` is what the page hands
    // the stream so the pill reads as a file count instead of `+N −M`.
    expect(dockPill(base)).toEqual({ tab: "changes", count: 3 });
  });

  test("documents carry the pill when the session wrote no files", () => {
    expect(dockPill({ ...base, changes: 0, artifacts: 2 })).toEqual({
      tab: "artifacts",
      count: 2,
    });
  });
});
