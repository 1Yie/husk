import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { ChangesList } from "../index";
import type { FileChange } from "@/features/chat/hooks/stream-view";

const change = (path: string, body: string): FileChange => ({
  path,
  kind: "update",
  patches: [{ n: 1, tool: "fuzzy_patch", diff: `@@ -1 +1 @@\n-old\n+${body}` }],
  adds: 1,
  dels: 1,
  fuzzy: false,
});

/** The diff body is syntax-highlighted, so a line's text is split across token
 *  spans — assert over the rendered text, not a single text node. */
const shows = (container: HTMLElement, text: string) =>
  container.textContent?.includes(text) ?? false;

test("a small change set still opens by default", () => {
  const { container } = render(<ChangesList changes={[change("one.ts", "one-new")]} />);
  expect(shows(container, "one-new")).toBe(true);
});

test("many files start collapsed and expand one row at a time", () => {
  const changes = Array.from({ length: 6 }, (_, i) => change(`f${i}.ts`, `new-${i}`));
  const { container } = render(<ChangesList changes={changes} />);
  expect(shows(container, "new-0")).toBe(false);
  fireEvent.click(screen.getByRole("button", { name: /f0\.ts/ }));
  expect(shows(container, "new-0")).toBe(true);
  expect(shows(container, "new-1")).toBe(false);
});

test("crossing the threshold mid-turn keeps the diffs already on screen open", () => {
  // The dock auto-opens on the first change, so the threshold is crossed
  // WHILE the reader is looking at an expanded diff. Folding the list at that
  // instant yanked the patch they were reading out from under them.
  const one = [change("a.ts", "a-new")];
  const { container, rerender } = render(<ChangesList changes={one} />);
  expect(shows(container, "a-new")).toBe(true);

  rerender(<ChangesList changes={[...one, change("b.ts", "b-new"), change("c.ts", "c-new"), change("d.ts", "d-new"), change("e.ts", "e-new")]} />);
  // Already-open row stays open…
  expect(shows(container, "a-new")).toBe(true);
  // …a file that arrives after the crossing comes in collapsed.
  expect(shows(container, "b-new")).toBe(false);
});

test("a list that shrinks below the threshold does not flip back to expanded", () => {
  const many = Array.from({ length: 6 }, (_, i) => change(`f${i}.ts`, `new-${i}`));
  const { container, rerender } = render(<ChangesList changes={many} />);
  rerender(<ChangesList changes={[many[0]]} />);
  // Sticky: a turn that rewrites fewer files must not remount every diff.
  expect(shows(container, "new-0")).toBe(false);
});
