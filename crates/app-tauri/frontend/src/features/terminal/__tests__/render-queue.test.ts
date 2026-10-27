import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { beginLayoutResize } from "@/lib/layout-resize";
import { queueTerminalWork } from "../render-queue";

let cancellations: (() => void)[];
beforeEach(() => { vi.useFakeTimers(); cancellations = []; });
afterEach(() => { for (const cancel of cancellations) cancel(); vi.useRealTimers(); });
function queue(start: () => void, foreground?: () => boolean) {
  const cancel = queueTerminalWork(start, foreground);
  cancellations.push(cancel);
  return cancel;
}

test("a batch of renderers initializes across separate frames", async () => {
  const calls: number[] = [];
  for (let i = 0; i < 3; i++) queue(() => calls.push(i));
  await vi.advanceTimersByTimeAsync(16);
  expect(calls).toEqual([0]);
  await vi.advanceTimersByTimeAsync(16);
  expect(calls).toEqual([0, 1]);
  await vi.advanceTimersByTimeAsync(16);
  expect(calls).toEqual([0, 1, 2]);
});

test("the latest focused pending pane initializes first", async () => {
  const calls: number[] = [];
  let active = 1;
  queue(() => calls.push(1), () => active === 1);
  queue(() => calls.push(2), () => active === 2);
  active = 2;
  await vi.advanceTimersByTimeAsync(16);
  expect(calls).toEqual([2]);
});

test("closed pending panes never create a renderer", async () => {
  const start = vi.fn();
  queue(start)();
  await vi.advanceTimersByTimeAsync(64);
  expect(start).not.toHaveBeenCalled();
});

test("pointer resizing pauses queued initialization until release", async () => {
  const start = vi.fn();
  const release = beginLayoutResize();
  try {
    queue(start);
    await vi.advanceTimersByTimeAsync(64);
    expect(start).not.toHaveBeenCalled();
  } finally { release(); }
  await vi.advanceTimersByTimeAsync(16);
  expect(start).toHaveBeenCalledOnce();
});
