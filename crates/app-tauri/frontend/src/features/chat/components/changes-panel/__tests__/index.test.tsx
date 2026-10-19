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

test("a small change set still opens by default", () => {
  render(<ChangesList changes={[change("one.ts", "one-new")]} />);
  expect(screen.getByText("one-new")).toBeInTheDocument();
});

test("many files start collapsed and expand one row at a time", () => {
  const changes = Array.from({ length: 6 }, (_, i) => change(`f${i}.ts`, `new-${i}`));
  render(<ChangesList changes={changes} />);
  expect(screen.queryByText("new-0")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /f0\.ts/ }));
  expect(screen.getByText("new-0")).toBeInTheDocument();
  expect(screen.queryByText("new-1")).not.toBeInTheDocument();
});

test("crossing the threshold mid-turn keeps the diffs already on screen open", () => {
  // The dock auto-opens on the first change, so the threshold is crossed
  // WHILE the reader is looking at an expanded diff. Folding the list at that
  // instant yanked the patch they were reading out from under them.
  const one = [change("a.ts", "a-new")];
  const { rerender } = render(<ChangesList changes={one} />);
  expect(screen.getByText("a-new")).toBeInTheDocument();

  rerender(<ChangesList changes={[...one, change("b.ts", "b-new"), change("c.ts", "c-new"), change("d.ts", "d-new"), change("e.ts", "e-new")]} />);
  // Already-open row stays open…
  expect(screen.getByText("a-new")).toBeInTheDocument();
  // …a file that arrives after the crossing comes in collapsed.
  expect(screen.queryByText("b-new")).not.toBeInTheDocument();
});

test("a list that shrinks below the threshold does not flip back to expanded", () => {
  const many = Array.from({ length: 6 }, (_, i) => change(`f${i}.ts`, `new-${i}`));
  const { rerender } = render(<ChangesList changes={many} />);
  rerender(<ChangesList changes={[many[0]]} />);
  // Sticky: a turn that rewrites fewer files must not remount every diff.
  expect(screen.queryByText("new-0")).not.toBeInTheDocument();
});
