import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { createTerminalResizePreview } from "../resize-preview";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

test("a burst of pointer inputs paints just the latest height in one frame", async () => {
  const paint = vi.fn();
  const preview = createTerminalResizePreview(paint);
  for (let height = 280; height <= 320; height++) preview.move(height);
  expect(paint).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(16);
  expect(paint).toHaveBeenCalledExactlyOnceWith(320, false);
  preview.cancel();
});

test("the split boundary follows frames without fitting terminal contents", async () => {
  const paint = vi.fn();
  const preview = createTerminalResizePreview(paint);
  for (const height of [280, 300, 320, 340]) {
    preview.move(height);
    await vi.advanceTimersByTimeAsync(16);
  }
  expect(paint.mock.calls.map(([height]) => height)).toEqual([280, 300, 320, 340]);
  expect(paint.mock.calls.filter(([, fit]) => fit)).toHaveLength(0);
  preview.cancel();
});

test("pausing mid-drag never changes terminal geometry", async () => {
  const paint = vi.fn();
  const preview = createTerminalResizePreview(paint);
  preview.move(280);
  await vi.advanceTimersByTimeAsync(16);
  preview.move(320);
  await vi.advanceTimersByTimeAsync(16);
  expect(paint).toHaveBeenLastCalledWith(320, false);
  await vi.advanceTimersByTimeAsync(80);
  expect(paint).toHaveBeenLastCalledWith(320, false);
  expect(paint).toHaveBeenCalledTimes(2);
  preview.cancel();
});

test("release flushes immediately and cancels pending preview work", async () => {
  const paint = vi.fn();
  const preview = createTerminalResizePreview(paint);
  preview.move(300);
  preview.flush(340);
  expect(paint).toHaveBeenCalledExactlyOnceWith(340, true);
  await vi.advanceTimersByTimeAsync(100);
  expect(paint).toHaveBeenCalledOnce();
});

test("cancel never paints an old size after the panel hides or unmounts", async () => {
  const paint = vi.fn();
  const preview = createTerminalResizePreview(paint);
  preview.move(300);
  preview.cancel();
  await vi.advanceTimersByTimeAsync(100);
  expect(paint).not.toHaveBeenCalled();
});
