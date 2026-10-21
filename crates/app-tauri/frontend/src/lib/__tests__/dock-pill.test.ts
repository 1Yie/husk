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
  const base = { filesTab: "changes" as const, changes: 3, artifacts: 0, changesLive: true };

  test("no files, no pill", () => {
    expect(dockPill({ ...base, changes: 0, changesLive: false })).toEqual({ tab: null, count: 0 });
  });

  test("the stored file view wins while it has content", () => {
    expect(dockPill({ ...base, artifacts: 2, filesTab: "artifacts" })).toEqual({
      tab: "artifacts",
      count: 2,
    });
    // Both lists have content and the reader last stood on 改动.
    expect(dockPill({ ...base, artifacts: 2 })).toEqual({ tab: "changes", count: 3 });
  });

  test("committed changes hand the pill to 产物", () => {
    expect(dockPill({ ...base, artifacts: 2, changesLive: false })).toEqual({
      tab: "artifacts",
      count: 2,
    });
  });

  test("committed changes with no documents hide the pill", () => {
    expect(dockPill({ ...base, changesLive: false })).toEqual({ tab: null, count: 0 });
  });
});
