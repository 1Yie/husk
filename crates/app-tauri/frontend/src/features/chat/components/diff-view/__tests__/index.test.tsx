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
