export type TerminalPane = { key: number; cwd?: string };

export type SplitDirection = "columns" | "rows";

export type PaneLayout =
  | { kind: "leaf"; pane: TerminalPane }
  | {
      kind: "split";
      key: number;
      direction: SplitDirection;
      ratio: number;
      first: PaneLayout;
      second: PaneLayout;
    };

export function listPanes(root: PaneLayout): TerminalPane[] {
  const panes: TerminalPane[] = [];
  function visit(node: PaneLayout): void {
    if (node.kind === "leaf") {
      panes.push(node.pane);
    } else {
      visit(node.first);
      visit(node.second);
    }
  }
  visit(root);
  return panes;
}

export function splitPane(
  root: PaneLayout,
  paneKey: number,
  newPane: TerminalPane,
  direction: SplitDirection,
): PaneLayout {
  if (root.kind === "leaf") {
    return root.pane.key === paneKey
      ? {
          kind: "split",
          key: newPane.key,
          direction,
          ratio: 0.5,
          first: root,
          second: { kind: "leaf", pane: newPane },
        }
      : root;
  }
  const first = splitPane(root.first, paneKey, newPane, direction);
  const second = splitPane(root.second, paneKey, newPane, direction);
  return first === root.first && second === root.second
    ? root
    : { ...root, first, second };
}

export function removePane(root: PaneLayout, paneKey: number): PaneLayout | null {
  if (root.kind === "leaf") return root.pane.key === paneKey ? null : root;
  const first = removePane(root.first, paneKey);
  const second = removePane(root.second, paneKey);
  if (!first) return second;
  if (!second) return first;
  return first === root.first && second === root.second
    ? root
    : { ...root, first, second };
}

export function resizeSplit(
  root: PaneLayout,
  splitKey: number,
  ratio: number,
): PaneLayout {
  if (!Number.isFinite(ratio) || root.kind === "leaf") return root;
  if (root.key === splitKey) {
    const clamped = Math.min(0.9, Math.max(0.1, ratio));
    return root.ratio === clamped ? root : { ...root, ratio: clamped };
  }
  const first = resizeSplit(root.first, splitKey, ratio);
  const second = resizeSplit(root.second, splitKey, ratio);
  return first === root.first && second === root.second
    ? root
    : { ...root, first, second };
}

export function layoutRectangles(root: PaneLayout): {
  panes: Array<{
    pane: TerminalPane;
    x: number;
    y: number;
    width: number;
    height: number;
  }>;
  splits: Array<{
    key: number;
    direction: SplitDirection;
    ratio: number;
    x: number;
    y: number;
    width: number;
    height: number;
  }>;
} {
  const panes: ReturnType<typeof layoutRectangles>["panes"] = [];
  const splits: ReturnType<typeof layoutRectangles>["splits"] = [];
  function visit(
    node: PaneLayout,
    x: number,
    y: number,
    width: number,
    height: number,
  ): void {
    if (node.kind === "leaf") {
      panes.push({ pane: node.pane, x, y, width, height });
      return;
    }
    splits.push({
      key: node.key,
      direction: node.direction,
      ratio: node.ratio,
      x,
      y,
      width,
      height,
    });
    if (node.direction === "columns") {
      const firstWidth = width * node.ratio;
      visit(node.first, x, y, firstWidth, height);
      visit(node.second, x + firstWidth, y, width - firstWidth, height);
    } else {
      const firstHeight = height * node.ratio;
      visit(node.first, x, y, width, firstHeight);
      visit(node.second, x, y + firstHeight, width, height - firstHeight);
    }
  }
  visit(root, 0, 0, 1, 1);
  return { panes, splits };
}
