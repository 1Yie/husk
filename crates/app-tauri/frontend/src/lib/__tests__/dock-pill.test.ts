// Which view the dock pill names, and how many files it counts.

import { describe, expect, test } from "vitest";
import { dockPill } from "../dock-pill";

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

  test("documents carry the pill when the session wrote no files", () => {
    expect(dockPill({ ...base, changes: 0, artifacts: 2 })).toEqual({
      tab: "artifacts",
      count: 2,
    });
  });
});
