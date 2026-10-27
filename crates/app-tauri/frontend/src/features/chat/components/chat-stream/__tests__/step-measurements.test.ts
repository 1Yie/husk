import { beforeEach, expect, test, vi } from "vitest";
import { createStepMeasurements, stepBatchScrollDelta } from "../step-measurements";

test("a preceding shrink must not misclassify a visible step as above the viewport", () => {
  expect(stepBatchScrollDelta([
    // Originally at -100, reaching INTO the viewport to +50. Its top has
    // moved to -180 because the preceding row lost 80px.
    { top: -180, previous: 150, height: 200 },
    { top: -200, previous: 100, height: 20 },
  ], 0)).toBe(-80);
});

test("a preceding growth must not hide another above-viewport correction", () => {
  expect(stepBatchScrollDelta([
    // Originally -150..-50, wholly above the viewport; the growth before it
    // moved its new top to +50. Observer entries arrive in reverse order.
    { top: 50, previous: 100, height: 150 },
    { top: -300, previous: 100, height: 300 },
  ], 0)).toBe(250);
});

test("fractional changes are summed before any rounding", () => {
  expect(stepBatchScrollDelta(
    Array.from({ length: 16 }, (_, i) => ({
      top: -1000 + i * 40.0625, previous: 36, height: 36.0625,
    })),
    0,
  )).toBe(1);
});

class BatchResizeObserver {
  static instance: BatchResizeObserver;
  constructor(private callback: ResizeObserverCallback) { BatchResizeObserver.instance = this; }
  observe() {}
  unobserve() {}
  disconnect() {}
  resize(target: Element, height: number) {
    this.batch([{ target, height }]);
  }
  batch(entries: { target: Element; height: number }[]) {
    this.callback(entries.map(({ target, height }) => ({
      target, borderBoxSize: [{ blockSize: height, inlineSize: 500 }],
    } as unknown as ResizeObserverEntry)), this as unknown as ResizeObserver);
  }
}

beforeEach(() => vi.stubGlobal("ResizeObserver", BatchResizeObserver));

function geometry() {
  const root = document.createElement("div");
  const step = document.createElement("div");
  root.append(step);
  document.body.append(root);
  root.getBoundingClientRect = () => ({ top: 0 } as DOMRect);
  step.getBoundingClientRect = () => ({ top: -500, height: 80 } as DOMRect);
  root.scrollTop = 1000;
  return { root, step };
}

test("history restoration consumes pending measurements without compensating twice", () => {
  const { root, step } = geometry();
  const measurements = createStepMeasurements(root, () => true);
  const changed = vi.fn();
  measurements.observe(step, 100, changed);
  // The history prepend already restored the viewport using the total
  // height delta. Settle before the pending observer delivery.
  measurements.settle();
  BatchResizeObserver.instance.resize(step, 80);
  expect(root.scrollTop).toBe(1000);
  expect(changed).toHaveBeenCalledWith(80);
  measurements.disconnect();
});

test("bottom-follow and smooth jumps still record sizes but do not fight their scroll", () => {
  const { root, step } = geometry();
  const measurements = createStepMeasurements(root, () => false);
  const changed = vi.fn();
  measurements.observe(step, 100, changed);
  BatchResizeObserver.instance.resize(step, 80);
  expect(root.scrollTop).toBe(1000);
  expect(changed).toHaveBeenCalledWith(80);
  measurements.disconnect();
});

test("a zero-height step removes its spacer instead of retaining an estimate", () => {
  const { root, step } = geometry();
  const measurements = createStepMeasurements(root, () => true);
  const changed = vi.fn();
  measurements.observe(step, 100, changed);
  BatchResizeObserver.instance.resize(step, 0);
  expect(root.scrollTop).toBe(900);
  expect(changed).toHaveBeenCalledWith(0);
  measurements.disconnect();
});

test("detached steps no longer contribute late measurements", () => {
  const { root, step } = geometry();
  const measurements = createStepMeasurements(root, () => true);
  const changed = vi.fn();
  const unobserve = measurements.observe(step, 100, changed);
  unobserve();
  BatchResizeObserver.instance.resize(step, 80);
  expect(root.scrollTop).toBe(1000);
  expect(changed).not.toHaveBeenCalled();
  measurements.disconnect();
});

test("collapsed zero-height nodes sharing a top are ordered by DOM, not callback order", () => {
  const { root, step } = geometry();
  const second = document.createElement("div");
  root.append(second);
  // Old first: -80..-30 (above). Old second: -30..+70 (visible).
  // Both collapse, so they now have the exact same top.
  step.getBoundingClientRect = second.getBoundingClientRect = () => ({ top: -80, height: 0 } as DOMRect);
  const measurements = createStepMeasurements(root, () => true);
  measurements.observe(step, 50, () => {});
  measurements.observe(second, 100, () => {});
  BatchResizeObserver.instance.batch([{ target: second, height: 0 }, { target: step, height: 0 }]);
  expect(root.scrollTop).toBe(950);
  measurements.disconnect();
});
