import { expect, test } from "vitest";
import {
  layoutRectangles,
  listPanes,
  removePane,
  resizeSplit,
  splitPane,
  type PaneLayout,
} from "../pane-layout";

function leaf(key: number, cwd?: string): PaneLayout {
  return Object.freeze({
    kind: "leaf",
    pane: Object.freeze({ key, cwd }),
  });
}

function branch(
  key: number,
  direction: "columns" | "rows",
  ratio: number,
  first: PaneLayout,
  second: PaneLayout,
): PaneLayout {
  return Object.freeze({ kind: "split", key, direction, ratio, first, second });
}

const first = leaf(1, "/first");
const second = leaf(2, "/second");
const third = leaf(3);
const fourth = leaf(4);
const bottom = branch(4, "columns", 0.5, third, fourth);
const right = branch(3, "rows", 0.25, second, bottom);
const root = branch(2, "columns", 0.5, first, right);

test("lists nested panes in first-to-second order without copying pane objects", () => {
  const panes = listPanes(root);
  expect(panes.map((pane) => pane.key)).toEqual([1, 2, 3, 4]);
  for (const [index, node] of [first, second, third, fourth].entries()) {
    if (node.kind !== "leaf") throw new Error("Expected fixture leaf");
    expect(panes[index]).toBe(node.pane);
  }
  expect(listPanes(first)).toEqual([panes[0]]);
});

test.each(["columns", "rows"] as const)(
  "splits a nested pane into %s while preserving all existing pane objects",
  (direction) => {
    const newPane = Object.freeze({ key: 5, cwd: "/new" });
    const updated = splitPane(root, 3, newPane, direction);
    expect(updated).not.toBe(root);
    expect(updated).toEqual({
      ...root,
      second: {
        ...right,
        second: {
          ...bottom,
          first: {
            kind: "split",
            key: 5,
            direction,
            ratio: 0.5,
            first: third,
            second: { kind: "leaf", pane: newPane },
          },
        },
      },
    });
    if (updated.kind !== "split" || updated.second.kind !== "split") {
      throw new Error("Expected nested split");
    }
    expect(updated.first).toBe(first);
    expect(updated.second.first).toBe(second);
    const updatedBottom = updated.second.second;
    if (updatedBottom.kind !== "split" || updatedBottom.first.kind !== "split") {
      throw new Error("Expected split at target");
    }
    expect(updatedBottom.second).toBe(fourth);
    expect(updatedBottom.first.first).toBe(third);
    const oldPanes = listPanes(root);
    const panes = listPanes(updated);
    expect(panes.map((pane) => pane.key)).toEqual([1, 2, 3, 5, 4]);
    for (const pane of oldPanes) {
      expect(panes.find((candidate) => candidate.key === pane.key)).toBe(pane);
    }
    expect(panes[3]).toBe(newPane);
    expect(listPanes(root).map((pane) => pane.key)).toEqual([1, 2, 3, 4]);
  },
);

test("splitting a single leaf keeps the old leaf and missing targets keep the root", () => {
  const newPane = { key: 5 };
  const updated = splitPane(first, 1, newPane, "rows");
  expect(updated).toEqual({
    kind: "split",
    key: 5,
    direction: "rows",
    ratio: 0.5,
    first,
    second: { kind: "leaf", pane: newPane },
  });
  if (updated.kind !== "split") throw new Error("Expected split");
  expect(updated.first).toBe(first);
  expect(splitPane(root, 99, newPane, "columns")).toBe(root);
  expect(splitPane(first, 99, newPane, "columns")).toBe(first);
});

test("removing either child collapses its split to the exact surviving node", () => {
  expect(removePane(bottom, 3)).toBe(fourth);
  expect(removePane(bottom, 4)).toBe(third);
  expect(removePane(root, 1)).toBe(right);
  expect(removePane(first, 1)).toBeNull();
});

test("nested removal copies only ancestors and preserves surviving pane objects", () => {
  const updated = removePane(root, 3);
  expect(updated).toEqual({
    ...root,
    second: { ...right, second: fourth },
  });
  if (updated?.kind !== "split" || updated.second.kind !== "split") {
    throw new Error("Expected surviving branches");
  }
  expect(updated).not.toBe(root);
  expect(updated.first).toBe(first);
  expect(updated.second.first).toBe(second);
  expect(updated.second.second).toBe(fourth);
  const panes = listPanes(updated);
  const originals = listPanes(root);
  expect(panes.map((pane) => pane.key)).toEqual([1, 2, 4]);
  expect(panes[0]).toBe(originals[0]);
  expect(panes[1]).toBe(originals[1]);
  expect(panes[2]).toBe(originals[3]);
  const withoutSecond = removePane(updated, 2);
  expect(withoutSecond).toEqual({ ...root, second: fourth });
  if (!withoutSecond) throw new Error("Expected remaining panes");
  expect(removePane(withoutSecond, 4)).toBe(first);
  expect(listPanes(root).map((pane) => pane.key)).toEqual([1, 2, 3, 4]);
});

test("removing missing pane keys preserves root and leaf identity", () => {
  expect(removePane(root, 99)).toBe(root);
  expect(removePane(first, 99)).toBe(first);
});

test("resizing a nested split shares children, siblings, and all pane objects", () => {
  const updated = resizeSplit(root, 4, 0.75);
  expect(updated).toEqual({
    ...root,
    second: { ...right, second: { ...bottom, ratio: 0.75 } },
  });
  if (updated.kind !== "split" || updated.second.kind !== "split") {
    throw new Error("Expected nested branches");
  }
  expect(updated.first).toBe(first);
  expect(updated.second.first).toBe(second);
  const resized = updated.second.second;
  if (resized.kind !== "split") throw new Error("Expected resized split");
  expect(resized.first).toBe(third);
  expect(resized.second).toBe(fourth);
  const originals = listPanes(root);
  listPanes(updated).forEach((pane, index) => expect(pane).toBe(originals[index]));
  expect(bottom.kind === "split" && bottom.ratio).toBe(0.5);
});

test.each([
  [-100, 0.1],
  [0, 0.1],
  [0.1, 0.1],
  [0.3, 0.3],
  [0.9, 0.9],
  [1, 0.9],
  [100, 0.9],
])("resizing clamps finite ratio %s to %s", (ratio, expected) => {
  const updated = resizeSplit(root, 2, ratio);
  expect(updated.kind === "split" && updated.ratio).toBe(expected);
  if (updated.kind !== "split") throw new Error("Expected split");
  expect(updated.first).toBe(first);
  expect(updated.second).toBe(right);
  expect(resizeSplit(updated, 2, ratio)).toBe(updated);
});

test("resize no-ops, missing keys, and nonfinite ratios preserve root identity", () => {
  expect(resizeSplit(root, 2, 0.5)).toBe(root);
  expect(resizeSplit(root, 4, 0.5)).toBe(root);
  expect(resizeSplit(root, 99, 0.7)).toBe(root);
  expect(resizeSplit(first, 1, 0.7)).toBe(first);
  for (const ratio of [NaN, Infinity, -Infinity]) {
    expect(resizeSplit(root, 2, ratio)).toBe(root);
    expect(resizeSplit(root, 4, ratio)).toBe(root);
  }
});

test("nested geometry uses normalized coordinates and entire branch split bounds", () => {
  const rectangles = layoutRectangles(root);
  const panes = listPanes(root);
  expect(rectangles.panes).toEqual([
    { pane: panes[0], x: 0, y: 0, width: 0.5, height: 1 },
    { pane: panes[1], x: 0.5, y: 0, width: 0.5, height: 0.25 },
    { pane: panes[2], x: 0.5, y: 0.25, width: 0.25, height: 0.75 },
    { pane: panes[3], x: 0.75, y: 0.25, width: 0.25, height: 0.75 },
  ]);
  expect(rectangles.splits).toEqual([
    { key: 2, direction: "columns", ratio: 0.5, x: 0, y: 0, width: 1, height: 1 },
    { key: 3, direction: "rows", ratio: 0.25, x: 0.5, y: 0, width: 0.5, height: 1 },
    { key: 4, direction: "columns", ratio: 0.5, x: 0.5, y: 0.25, width: 0.5, height: 0.75 },
  ]);
  rectangles.panes.forEach(({ pane }, index) => expect(pane).toBe(panes[index]));
});

test("a lone pane occupies the full layout with no splits", () => {
  const rectangles = layoutRectangles(first);
  expect(rectangles).toEqual({
    panes: [{ pane: listPanes(first)[0], x: 0, y: 0, width: 1, height: 1 }],
    splits: [],
  });
});

test("geometry follows resized ratios and expands surviving panes after removal", () => {
  const resized = resizeSplit(root, 3, 0.5);
  const rectangles = layoutRectangles(resized);
  expect(rectangles.panes[1]).toMatchObject({ x: 0.5, y: 0, width: 0.5, height: 0.5 });
  expect(rectangles.panes[2]).toMatchObject({ x: 0.5, y: 0.5, width: 0.25, height: 0.5 });
  const remaining = removePane(resized, 1);
  if (!remaining) throw new Error("Expected surviving branch");
  expect(layoutRectangles(remaining).panes[0]).toMatchObject({
    x: 0,
    y: 0,
    width: 1,
    height: 0.5,
  });
});
