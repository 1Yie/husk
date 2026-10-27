import { useCallback, useEffect, useRef } from "react";
import { beginLayoutResize } from "@/lib/layout-resize";
import { layoutRectangles, resizeSplit, type PaneLayout, type SplitDirection } from "./pane-layout";

type SplitRect = ReturnType<typeof layoutRectangles>["splits"][number];
export const MIN_PANE_WIDTH = 120;
export const MIN_PANE_HEIGHT = 70;

function paneBox(rect: { x: number; y: number; width: number; height: number }) {
  return { left: `${rect.x * 100}%`, top: `${rect.y * 100}%`, width: `${rect.width * 100}%`, height: `${rect.height * 100}%` };
}
function splitBox(rect: SplitRect) {
  return rect.direction === "columns"
    ? { left: `${(rect.x + rect.width * rect.ratio) * 100}%`, top: `${rect.y * 100}%`, height: `${rect.height * 100}%` }
    : { left: `${rect.x * 100}%`, top: `${(rect.y + rect.height * rect.ratio) * 100}%`, width: `${rect.width * 100}%` };
}
function minimum(node: PaneLayout, direction: SplitDirection): number {
  if (node.kind === "leaf") return direction === "columns" ? MIN_PANE_WIDTH : MIN_PANE_HEIGHT;
  const first = minimum(node.first, direction), second = minimum(node.second, direction);
  // Descendants keep their ratios when an ancestor moves, so an uneven
  // branch needs enough space for its smallest allocated fraction.
  return node.direction === direction
    ? Math.max(first / node.ratio, second / (1 - node.ratio))
    : Math.max(first, second);
}
function findSplit(node: PaneLayout, key: number): Extract<PaneLayout, { kind: "split" }> | undefined {
  if (node.kind === "leaf") return;
  return node.key === key ? node : findSplit(node.first, key) ?? findSplit(node.second, key);
}
function ratioBounds(layout: PaneLayout, split: SplitRect, bounds: Pick<DOMRect, "width" | "height">) {
  const node = findSplit(layout, split.key);
  const available = split.direction === "columns" ? bounds.width * split.width : bounds.height * split.height;
  if (!node || available <= 0) return { min: .1, max: .9 };
  const first = minimum(node.first, split.direction) / available;
  const second = minimum(node.second, split.direction) / available;
  const min = Math.max(.1, first), max = Math.min(.9, 1 - second);
  return min <= max ? { min, max } : { min: split.ratio, max: split.ratio };
}
function paint(root: HTMLElement, layout: PaneLayout) {
  const rectangles = layoutRectangles(layout);
  for (const rect of rectangles.panes) {
    const element = root.querySelector<HTMLElement>(`[data-pane-key="${rect.pane.key}"]`);
    if (element) Object.assign(element.style, paneBox(rect));
  }
  for (const rect of rectangles.splits) {
    const element = root.querySelector<HTMLElement>(`[data-split-key="${rect.key}"]`);
    if (element) {
      Object.assign(element.style, splitBox(rect));
      element.setAttribute("aria-valuenow", String(Math.round(rect.ratio * 100)));
    }
  }
}

export function SplitHandle({ layout, split, groupKey, active, onResize }: {
  layout: PaneLayout; split: SplitRect; groupKey: number; active: boolean;
  onResize: (group: number, split: number, ratio: number) => void;
}) {
  const handle = useRef<HTMLDivElement>(null);
  const drag = useRef<{ pointer: number; root: HTMLElement; layout: PaneLayout; bounds: DOMRect;
    ratio: number; min: number; max: number; release: () => void }>();
  const frame = useRef<number>();
  const clamp = (ratio: number, min = .1, max = .9) => Math.max(min, Math.min(max, ratio));
  const finish = useCallback(() => {
    const current = drag.current;
    if (!current) return;
    if (frame.current !== undefined) cancelAnimationFrame(frame.current);
    frame.current = undefined;
    paint(current.root, resizeSplit(current.layout, split.key, current.ratio));
    drag.current = undefined;
    handle.current?.removeAttribute("data-dragging");
    onResize(groupKey, split.key, current.ratio);
    current.release();
  }, [groupKey, onResize, split.key]);
  useEffect(() => { if (!active) finish(); }, [active, finish]);
  useEffect(() => () => {
    if (frame.current !== undefined) cancelAnimationFrame(frame.current);
    drag.current?.release();
    drag.current = undefined;
  }, []);
  const update = (x: number, y: number) => {
    const current = drag.current;
    if (!current) return;
    const ratio = split.direction === "columns"
      ? ((x - current.bounds.left) / current.bounds.width - split.x) / split.width
      : ((y - current.bounds.top) / current.bounds.height - split.y) / split.height;
    current.ratio = clamp(ratio, current.min, current.max);
  };
  return (
    <div ref={handle} role="separator"
      aria-label={split.direction === "columns" ? "调整终端分屏宽度" : "调整终端分屏高度"}
      aria-orientation={split.direction === "columns" ? "vertical" : "horizontal"}
      aria-valuemin={10} aria-valuemax={90} aria-valuenow={Math.round(split.ratio * 100)}
      tabIndex={active ? 0 : -1} className="terminal-panel__splitter" data-direction={split.direction}
      data-split-key={split.key} style={splitBox(split)}
      onPointerDown={(event) => {
        if (event.button !== 0 || !active || drag.current) return;
        const root = event.currentTarget.closest<HTMLElement>(".terminal-panel__group");
        if (!root) return;
        const bounds = root.getBoundingClientRect();
        if (!bounds.width || !bounds.height) return;
        event.preventDefault();
        event.currentTarget.focus();
        event.currentTarget.setPointerCapture(event.pointerId);
        const limits = ratioBounds(layout, split, bounds);
        drag.current = { pointer: event.pointerId, root, layout, bounds, ratio: split.ratio,
          ...limits,
          release: beginLayoutResize() };
        event.currentTarget.setAttribute("data-dragging", "");
      }}
      onPointerMove={(event) => {
        if (drag.current?.pointer !== event.pointerId) return;
        update(event.clientX, event.clientY);
        if (frame.current !== undefined) return;
        frame.current = requestAnimationFrame(() => {
          frame.current = undefined;
          const current = drag.current;
          if (current) paint(current.root, resizeSplit(current.layout, split.key, current.ratio));
        });
      }}
      onPointerUp={(event) => {
        if (drag.current?.pointer !== event.pointerId) return;
        update(event.clientX, event.clientY);
        finish();
        if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
      }}
      onPointerCancel={finish} onLostPointerCapture={finish}
      onDoubleClick={(event) => {
        if (!active) return;
        const root = event.currentTarget.closest<HTMLElement>(".terminal-panel__group");
        if (!root) return;
        const limits = ratioBounds(layout, split, root.getBoundingClientRect());
        onResize(groupKey, split.key, clamp(.5, limits.min, limits.max));
      }}
      onKeyDown={(event) => {
        if (!active) return;
        const root = event.currentTarget.closest<HTMLElement>(".terminal-panel__group");
        if (!root) return;
        const limits = ratioBounds(layout, split, root.getBoundingClientRect());
        const next = event.key === "Home" ? limits.min : event.key === "End" ? limits.max
          : (split.direction === "columns" ? event.key === "ArrowLeft" : event.key === "ArrowUp") ? split.ratio - .05
          : (split.direction === "columns" ? event.key === "ArrowRight" : event.key === "ArrowDown") ? split.ratio + .05 : undefined;
        if (next === undefined || drag.current) return;
        event.preventDefault();
        const release = beginLayoutResize();
        onResize(groupKey, split.key, clamp(next, limits.min, limits.max));
        release();
      }}
    />
  );
}
