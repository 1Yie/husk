// The turn rail sizes itself from the lane it occupies, not from the window:
// a wrong height used to draw the strip over the composer and swallow clicks.

import type { ComponentProps } from "react";
import { fireEvent } from "@testing-library/react";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { ChatTurnRail, type RailMark } from "@/features/chat/components/chat-stream/chat-turn-rail";
import { renderWithProviders } from "@/test/render";

const ROW_H = 11;
/** Track padding the budget subtracts. */
const TRACK_PAD = 16;
const LANE_H = 200;
/** Rows that fit the 200px lane. */
const FITTING_ROWS = Math.floor((LANE_H - TRACK_PAD) / ROW_H);

/** Height the doubles report; `null` = the observer never delivers a box. */
let reportedHeight: number | null = LANE_H;

class SizingResizeObserver implements ResizeObserver {
  private callback: ResizeObserverCallback;
  constructor(callback: ResizeObserverCallback) {
    this.callback = callback;
  }
  observe(target: Element) {
    if (reportedHeight == null) return;
    const entry = { target, contentRect: { height: reportedHeight } as DOMRectReadOnly };
    this.callback([entry as ResizeObserverEntry], this);
  }
  unobserve() {}
  disconnect() {}
}

beforeEach(() => {
  reportedHeight = LANE_H;
  vi.stubGlobal("ResizeObserver", SizingResizeObserver);
});

function railMarks(count: number): RailMark[] {
  const inner = Array.from({ length: count }, (_, i): RailMark => ({
    id: `t${i}`,
    type: i % 2 === 0 ? "user" : "assistant",
    targetId: `c${i}`,
    previewTitle: `第 ${i} 轮`,
    previewSnippet: "",
  }));
  return [
    { id: "top", type: "top", targetId: "top", previewTitle: "顶部", previewSnippet: "" },
    ...inner,
    { id: "end", type: "end", targetId: "end", previewTitle: "底部", previewSnippet: "" },
  ];
}

function renderRail(overrides: Partial<ComponentProps<typeof ChatTurnRail>> = {}) {
  return renderWithProviders(
    <ChatTurnRail
      marks={railMarks(40)}
      activeId="t4"
      onSelectMark={() => {}}
      {...overrides}
    />,
  );
}

/** The lane's rows — every one is an 11px × 20px hit row. */
function rowsOf(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>('[class*="h-[11px]"]'));
}

describe("turn rail", () => {
  test("claims only the rows its lane has", () => {
    const { container } = renderRail();
    const root = container.firstElementChild as HTMLElement;
    const rows = rowsOf(root);
    expect(rows.length).toBeGreaterThan(0);
    expect(rows.length).toBeLessThanOrEqual(FITTING_ROWS);
    // The measured budget is in force, not the small first guess.
    expect(rows.length).toBeGreaterThan(9);
  });

  test("starts small until something measures the lane", () => {
    reportedHeight = null;
    const { container } = renderRail();
    const root = container.firstElementChild as HTMLElement;
    // A conservative first guess can never overshoot the container.
    expect(rowsOf(root).length).toBeLessThanOrEqual(9);
  });

  test("the lane itself takes no pointer, so the stream below it stays usable", () => {
    const { container } = renderRail();
    const root = container.firstElementChild as HTMLElement;
    expect(root.className).toContain("pointer-events-none");
  });

  test("the lane ends at the composer band", () => {
    const { container } = renderRail({ bottomInset: 120 });
    const root = container.firstElementChild as HTMLElement;
    expect(root).toHaveStyle({ bottom: "120px" });
  });

  test("a mark still jumps the conversation", () => {
    const onSelectMark = vi.fn();
    const marks = railMarks(40);
    const { container } = renderRail({ marks, onSelectMark });
    const root = container.firstElementChild as HTMLElement;
    const rows = rowsOf(root);
    // The last row is always the `end` handle.
    fireEvent.click(rows[rows.length - 1]);
    expect(onSelectMark).toHaveBeenCalledTimes(1);
    expect(onSelectMark.mock.calls[0][0]).toMatchObject({ id: "end", type: "end" });
  });

  test("the loading strip sits in the same lane", () => {
    const { container } = renderRail({ loading: true, bottomInset: 96 });
    const root = container.firstElementChild as HTMLElement;
    expect(root.className).toContain("pointer-events-none");
    expect(root).toHaveStyle({ bottom: "96px" });
    expect(rowsOf(root).length).toBeGreaterThan(0);
  });
});
