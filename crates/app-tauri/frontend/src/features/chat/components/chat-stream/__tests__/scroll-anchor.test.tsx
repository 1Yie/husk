// A step that changes height above the reader must not move the text being
// read: WebKit has no CSS scroll anchoring, so the stream does it. Without
// this, a slow scroll up mounted steps in the overscan at their real height
// instead of the estimate, and every correction twitched the content.

import { act, fireEvent } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
  ChatStream,
  scrollAnchorDelta,
} from "@/features/chat/components/chat-stream";
import { applyEvents } from "@/features/chat/hooks/apply-event";
import { emptyView } from "@/features/chat/hooks/stream-view";
import { renderWithProviders } from "@/test/render";

describe("scrollAnchorDelta", () => {
  test("a change wholly above the viewport is given back", () => {
    // Step at 0..100 above a viewport starting at 300 grows to 160.
    expect(scrollAnchorDelta(0, 100, 160, 300)).toBe(60);
    expect(scrollAnchorDelta(0, 100, 40, 300)).toBe(-60);
  });

  test("a step the reader is looking at grows in place", () => {
    // Its old bottom (250 + 100) reaches into the viewport at 300.
    expect(scrollAnchorDelta(250, 100, 160, 300)).toBe(0);
    // Wholly below the viewport top.
    expect(scrollAnchorDelta(400, 100, 160, 300)).toBe(0);
  });

  test("a step ending exactly at the viewport top counts as above", () => {
    expect(scrollAnchorDelta(200, 100, 130, 300)).toBe(30);
  });

  test("no change, no correction", () => {
    expect(scrollAnchorDelta(0, 100, 100, 300)).toBe(0);
  });
});

/** Placeholders until told otherwise: the shared double reports everything
 *  visible, which would mount every step on observe. */
class ManualIntersectionObserver {
  static instances: ManualIntersectionObserver[] = [];
  targets = new Set<Element>();
  constructor(private callback: IntersectionObserverCallback) {
    ManualIntersectionObserver.instances.push(this);
  }
  observe(target: Element) {
    this.targets.add(target);
  }
  unobserve(target: Element) {
    this.targets.delete(target);
  }
  disconnect() {
    this.targets.clear();
  }
  takeRecords() {
    return [];
  }
  show(target: Element) {
    this.callback(
      [{ target, isIntersecting: true } as IntersectionObserverEntry],
      this as unknown as IntersectionObserver,
    );
  }
  hide(target: Element) {
    this.callback(
      [{ target, isIntersecting: false } as IntersectionObserverEntry],
      this as unknown as IntersectionObserver,
    );
  }
}

class SilentResizeObserver {
  static instances = new Set<SilentResizeObserver>();
  targets = new Set<Element>();
  constructor(private callback: ResizeObserverCallback) {
    SilentResizeObserver.instances.add(this);
  }
  observe(target: Element) { this.targets.add(target); }
  unobserve(target: Element) { this.targets.delete(target); }
  disconnect() {
    this.targets.clear();
    SilentResizeObserver.instances.delete(this);
  }
  static resize(targets: Element[]) {
    for (const observer of this.instances) {
      const entries = targets.filter((target) => observer.targets.has(target))
        .map((target) => ({ target, contentRect: target.getBoundingClientRect() } as ResizeObserverEntry));
      if (entries.length) observer.callback(entries, observer as unknown as ResizeObserver);
    }
  }
}

/** jsdom has no layout: each element reports the rect this map gives it. */
const rects = new Map<Element, { top: number; height: number }>();
const realRect = HTMLElement.prototype.getBoundingClientRect;

beforeEach(() => {
  ManualIntersectionObserver.instances = [];
  SilentResizeObserver.instances.clear();
  rects.clear();
  vi.stubGlobal("IntersectionObserver", ManualIntersectionObserver);
  vi.stubGlobal("ResizeObserver", SilentResizeObserver);
  HTMLElement.prototype.getBoundingClientRect = function (this: HTMLElement) {
    // Steps start far above the viewport, so the first-layout check leaves
    // them as placeholders.
    const r =
      rects.get(this) ??
      (this.id.includes("-step-")
        ? { top: -5000, height: parseFloat(this.style.height) || 0 }
        : undefined);
    if (!r) return realRect.call(this);
    return {
      top: r.top,
      height: r.height,
      bottom: r.top + r.height,
      left: 0,
      right: 0,
      width: 0,
      x: 0,
      y: r.top,
      toJSON() {},
    } as DOMRect;
  };
});
afterEach(() => {
  HTMLElement.prototype.getBoundingClientRect = realRect;
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

test("a step mounting above the viewport keeps the reader's text in place", () => {
  const view = applyEvents(emptyView(), [
    { UserPrompt: "first" },
    {
      TextDelta: {
        text: "an answer long enough to need a placeholder estimate ".repeat(
          20,
        ),
      },
    },
    { StateChanged: "Finished" },
  ]);
  const { container } = renderWithProviders(
    <ChatStream view={view} sessionKey="/project:anchor" />,
  );
  const scroller = container.querySelector(".stream-scroll") as HTMLElement;
  const placeholder = container.querySelector(
    "[data-step-placeholder]",
  ) as HTMLElement | null;
  expect(placeholder).not.toBeNull();

  // The reader is below it, has scrolled away from the bottom.
  rects.set(scroller, { top: 0, height: 600 });
  scroller.scrollTop = 1000;
  Object.defineProperty(scroller, "scrollHeight", {
    value: 5000,
    configurable: true,
  });
  Object.defineProperty(scroller, "clientHeight", {
    value: 600,
    configurable: true,
  });
  fireEvent.wheel(scroller);
  fireEvent.scroll(scroller);

  // It mounts in the overscan, well above the viewport, 90px taller than
  // its placeholder was.
  const estimate = parseFloat(placeholder!.style.height);
  rects.set(placeholder!, { top: -800, height: estimate + 90 });
  act(() => {
    for (const io of ManualIntersectionObserver.instances)
      io.show(placeholder!);
  });
  act(() => SilentResizeObserver.resize([placeholder!]));
  expect(placeholder!.hasAttribute("data-step-mounted")).toBe(true);
  expect(scroller.scrollTop).toBe(1090);
});

test("a mixed batch reads layout first and corrects scrollTop only once", () => {
  const view = applyEvents(emptyView(), [
    { UserPrompt: "检查混排步骤" },
    { ReasoningDelta: { text: "先检查代码" } },
    { ToolCallStarted: { name: "bash", args_preview: "cargo test" } },
    { ToolCallFinished: { name: "bash", ok: true, content: "ok" } },
    { ReasoningDelta: { text: "再检查边界情况" } },
    { TextDelta: { text: "检查完成" } },
    { StateChanged: "Finished" },
  ]);
  const { container } = renderWithProviders(
    <ChatStream view={view} sessionKey="/project:mixed-batch" />,
  );
  const scroller = container.querySelector(".stream-scroll") as HTMLElement;
  const steps = [...container.querySelectorAll<HTMLElement>("[data-step-placeholder]")];
  expect(steps.length).toBeGreaterThanOrEqual(3);
  rects.set(scroller, { top: 0, height: 600 });
  Object.defineProperty(scroller, "scrollHeight", { value: 5000, configurable: true });
  Object.defineProperty(scroller, "clientHeight", { value: 600, configurable: true });
  scroller.scrollTop = 1000;
  fireEvent.wheel(scroller);
  fireEvent.scroll(scroller);

  for (let i = 0; i < steps.length; i++) {
    rects.set(steps[i], { top: -900 + i * 100, height: 36 });
  }
  act(() => {
    for (const io of ManualIntersectionObserver.instances) {
      for (const step of steps) io.show(step);
    }
  });
  // Deliver the first measurements, as a browser does before paint.
  act(() => SilentResizeObserver.resize(steps));

  let scrollTop = 1000;
  const writes: number[] = [];
  Object.defineProperty(scroller, "scrollTop", {
    configurable: true,
    get: () => scrollTop,
    set: (value: number) => { writes.push(value); scrollTop = value; },
  });
  rects.set(steps[0], { top: -900, height: 80 });
  rects.set(steps[1], { top: -756, height: 60 });
  // Observer delivery order is not guaranteed to follow DOM order.
  act(() => SilentResizeObserver.resize([steps[1], steps[0]]));
  expect(scrollTop).toBe(1068);
  expect(writes).toEqual([1068]);
});

test("an old offscreen notification cannot unmount a step now inside the viewport", () => {
  vi.useFakeTimers();
  const view = applyEvents(emptyView(), [
    { UserPrompt: "滚动时不要卸载可见内容" },
    { TextDelta: { text: "内容" } },
    { StateChanged: "Finished" },
  ]);
  const { container } = renderWithProviders(
    <ChatStream view={view} sessionKey="/project:visibility" />,
  );
  const scroller = container.querySelector(".stream-scroll") as HTMLElement;
  const step = container.querySelector("[data-step-placeholder]") as HTMLElement;
  rects.set(scroller, { top: 0, height: 600 });
  Object.defineProperty(scroller, "scrollHeight", { value: 5000, configurable: true });
  Object.defineProperty(scroller, "clientHeight", { value: 600, configurable: true });
  scroller.scrollTop = 1000;
  fireEvent.wheel(scroller);
  fireEvent.scroll(scroller);
  rects.set(step, { top: 100, height: 36.5 });
  act(() => {
    for (const io of ManualIntersectionObserver.instances) io.show(step);
  });
  act(() => {
    for (const io of ManualIntersectionObserver.instances) io.hide(step);
  });
  act(() => vi.advanceTimersByTime(1300));
  expect(step.hasAttribute("data-step-mounted")).toBe(true);

  // Truly outside the window: it should still unmount, at its exact size.
  rects.set(step, { top: -5000, height: 36.5 });
  act(() => {
    for (const io of ManualIntersectionObserver.instances) io.hide(step);
    vi.advanceTimersByTime(1300);
  });
  expect(step.hasAttribute("data-step-placeholder")).toBe(true);
  expect(step.style.height).toBe("36.5px");
});
