import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { DiffView } from "../index";

const diff = [
  "diff --git a/src/auth.ts b/src/auth.ts",
  "index 1a2b3c4..5d6e7f8 100644",
  "--- a/src/auth.ts",
  "+++ b/src/auth.ts",
  "@@ -12,3 +12,4 @@ export function getToken() {",
  " export function getToken() {",
  "-  return localStorage.token;",
  '+  const t = cookies.get("session");',
  '+  if (!t) throw new Error("no session");',
  "+  return t;",
  " }",
].join("\n");

test("a file's card names its path and counts its added and removed lines", () => {
  render(<DiffView diff={diff} />);
  expect(screen.getByText("src/auth.ts")).toBeInTheDocument();
  expect(screen.getByText("+3")).toBeInTheDocument();
  expect(screen.getByText("-1")).toBeInTheDocument();
});

test("the two gutter columns follow the hunk's own counters", () => {
  render(<DiffView diff={diff} />);
  // Context 12/12, deletion 13/–, additions –/13..15, closing context 14/16.
  expect(screen.getAllByText("12")).toHaveLength(2);
  expect(screen.getAllByText("13")).toHaveLength(2);
  expect(screen.getAllByText("14")).toHaveLength(2);
  expect(screen.getAllByText("15")).toHaveLength(1);
  expect(screen.getAllByText("16")).toHaveLength(1);
});

test("a multi-file patch becomes one card per file", () => {
  const patch = [
    "diff --git a/a.ts b/a.ts",
    "--- a/a.ts",
    "+++ b/a.ts",
    "@@ -1 +1 @@",
    "-one",
    "+two",
    "diff --git a/b.rs b/b.rs",
    "--- a/b.rs",
    "+++ b/b.rs",
    "@@ -1 +1 @@",
    "-three",
    "+four",
  ].join("\n");
  render(<DiffView diff={patch} />);
  expect(screen.getByText("a.ts")).toBeInTheDocument();
  expect(screen.getByText("b.rs")).toBeInTheDocument();
});

test("every card of a multi-file patch caps and scrolls itself", () => {
  // Under one shared vertical box, a tall card's horizontal bar sat at its
  // far end — out of reach until the box had scrolled past the whole file.
  const patch = [
    "diff --git a/a.ts b/a.ts",
    "@@ -1 +1 @@",
    "-one",
    "+two",
    "diff --git a/b.rs b/b.rs",
    "@@ -1 +1 @@",
    "-three",
    "+four",
  ].join("\n");
  const { container } = render(<DiffView diff={patch} maxHeight={300} />);
  const cards = [...container.querySelectorAll<HTMLElement>(".rounded-\\[12px\\]")];
  expect(cards).toHaveLength(2);
  for (const card of cards) expect(card.style.maxHeight).toBe("300px");
  // No ancestor between the cards and the caller scrolls on its own.
  expect(container.querySelector(".overflow-y-auto")?.closest(".rounded-\\[12px\\]")).toBeTruthy();
});

test("a hunk-only diff with no file header still numbers its lines", () => {
  // The dock hands each card a patch sliced down to its hunks.
  render(<DiffView diff={"@@ -0,0 +1,2 @@\n+first\n+second"} file="a.ts" />);
  expect(screen.getByText("1")).toBeInTheDocument();
  expect(screen.getByText("2")).toBeInTheDocument();
});

test("the scrollbar's room is held before any line overflows", () => {
  // A bar that appears later (highlight widening a line) must not grow the
  // card under the reader — the space is already there as padding.
  const { container } = render(<DiffView diff={diff} />);
  const body = container.querySelector(".overflow-y-auto")!;
  expect(body.className).toMatch(/\boverflow-x-hidden\b/);
  expect(body.className).toMatch(/\bpb-1\.5\b/);
});

test("the line-number gutter stays put while the code scrolls sideways", () => {
  render(<DiffView diff={diff} />);
  const gutter = screen.getAllByText("12")[0].parentElement!;
  expect(gutter.className).toMatch(/\bsticky\b/);
  expect(gutter.className).toMatch(/\bleft-0\b/);
});

test("apply_patch's a/file placeholder is not a filename", () => {
  const patch = ["--- a/file", "+++ b/file", "@@ -1 +1 @@", "-old", "+new"].join("\n");
  const { rerender } = render(<DiffView diff={patch} />);
  expect(screen.queryByText("file")).not.toBeInTheDocument();

  // The dock slices a patch down to its hunks and hands the card the path —
  // the card names itself instead of falling back to the `a/file` placeholder.
  rerender(<DiffView diff={patch} file="src/real.ts" />);
  expect(screen.getByText("src/real.ts")).toBeInTheDocument();
});

test("a huge diff mounts only the first 200 rows until expanded", () => {
  const big = [
    "@@ -1 +1,220 @@",
    ...Array.from({ length: 220 }, (_, i) => `+line-${i}`),
  ].join("\n");
  render(<DiffView diff={big} />);
  expect(screen.queryByText("line-219")).not.toBeInTheDocument();
  // 220 additions plus their hunk header — 200 rows fit, 21 wait behind the button.
  expect(screen.getByText("… 还有 21 行，点击展开全部")).toBeInTheDocument();
});
