import { expect, test } from "vitest";
import { recordFileChanges } from "@/features/chat/hooks/apply-event";
import type { FileChange } from "@/features/chat/hooks/stream-view";

const fuzzy = (path: string, body: string) =>
  `patched ${path} (line 1)\n\n@@ -1 +1 @@\n-old\n+${body}`;

test("a write replaces its own entry and leaves the previous view untouched", () => {
  const before: FileChange[] = [];
  recordFileChanges(before, "fuzzy_patch", fuzzy("a.ts", "a1"), false);
  recordFileChanges(before, "fuzzy_patch", fuzzy("b.ts", "b1"), false);
  const [a, b] = before;

  // The reducer hands in a shallow copy; the entries are still the old view's.
  const after = [...before];
  recordFileChanges(after, "fuzzy_patch", fuzzy("a.ts", "a2"), false);

  expect(after[0]).not.toBe(a);
  expect(after[0].patches).toHaveLength(2);
  expect(after[0].adds).toBe(2);
  // The untouched file keeps its identity, so its memoized row skips.
  expect(after[1]).toBe(b);
  // The previous view never sees the new patch.
  expect(a.patches).toHaveLength(1);
  expect(a.adds).toBe(1);
});

test("the approval's second landing only flips pending, on a fresh entry", () => {
  const staged: FileChange[] = [];
  recordFileChanges(staged, "fuzzy_patch", fuzzy("a.ts", "x"), false, true);
  const preview = staged[0];
  expect(preview.pending).toBe(true);

  const committed = [...staged];
  recordFileChanges(committed, "fuzzy_patch", fuzzy("a.ts", "x"), false);
  expect(committed[0]).not.toBe(preview);
  expect(committed[0].pending).toBe(false);
  expect(committed[0].patches).toHaveLength(1);
  expect(preview.pending).toBe(true);

  // Landing again with nothing new keeps the entry as it is.
  const again = [...committed];
  recordFileChanges(again, "fuzzy_patch", fuzzy("a.ts", "x"), false);
  expect(again[0]).toBe(committed[0]);
});
