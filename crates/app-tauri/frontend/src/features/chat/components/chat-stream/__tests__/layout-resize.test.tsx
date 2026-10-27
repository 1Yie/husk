import { act, fireEvent } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { ChatStream } from "..";
import { applyEvents } from "@/features/chat/hooks/apply-event";
import { emptyView } from "@/features/chat/hooks/stream-view";
import { beginLayoutResize } from "@/lib/layout-resize";
import { renderWithProviders } from "@/test/render";
import { TestResizeObserver } from "@/test/dom";

let release: (() => void) | undefined;
afterEach(() => { release?.(); release = undefined; vi.useRealTimers(); });

test("splitter movement defers chat bottom-follow until release", async () => {
  vi.useFakeTimers();
  const view = applyEvents(emptyView(), [
    { UserPrompt: "检查拖动时的布局" }, { TextDelta: { text: "回答" } }, { StateChanged: "Finished" },
  ]);
  const { container } = renderWithProviders(<ChatStream view={view} sessionKey="/resize:pinned" />);
  await act(async () => { await vi.advanceTimersByTimeAsync(500); });
  const scroller = container.querySelector(".stream-scroll") as HTMLElement;
  const scroll = vi.mocked(scroller.scrollTo);
  scroll.mockClear();
  release = beginLayoutResize();
  act(() => TestResizeObserver.resize(scroller));
  await act(async () => { await vi.advanceTimersByTimeAsync(32); });
  expect(scroll).not.toHaveBeenCalled();
  act(() => release!());
  await act(async () => { await vi.advanceTimersByTimeAsync(32); });
  expect(scroll).toHaveBeenCalledExactlyOnceWith({ top: scroller.scrollHeight, behavior: "instant" });
});

test("release does not repin a reader who had scrolled away", async () => {
  vi.useFakeTimers();
  const view = applyEvents(emptyView(), [
    { UserPrompt: "保留阅读位置" }, { TextDelta: { text: "回答" } }, { StateChanged: "Finished" },
  ]);
  const { container } = renderWithProviders(<ChatStream view={view} sessionKey="/resize:reader" />);
  await act(async () => { await vi.advanceTimersByTimeAsync(500); });
  const scroller = container.querySelector(".stream-scroll") as HTMLElement;
  Object.defineProperty(scroller, "scrollHeight", { value: 5000, configurable: true });
  Object.defineProperty(scroller, "clientHeight", { value: 600, configurable: true });
  scroller.scrollTop = 1000;
  fireEvent.wheel(scroller);
  fireEvent.scroll(scroller);
  const scroll = vi.mocked(scroller.scrollTo);
  scroll.mockClear();
  release = beginLayoutResize();
  act(() => TestResizeObserver.resize(scroller));
  act(() => release!());
  await act(async () => { await vi.advanceTimersByTimeAsync(32); });
  expect(scroll).not.toHaveBeenCalled();
});

test("a preview's temporary scroll clamp cannot turn bottom-follow back on", async () => {
  vi.useFakeTimers();
  const view = applyEvents(emptyView(), [
    { UserPrompt: "保留预览前的阅读锚点" }, { TextDelta: { text: "回答" } }, { StateChanged: "Finished" },
  ]);
  const { container } = renderWithProviders(<ChatStream view={view} sessionKey="/resize:clamp" />);
  await act(async () => { await vi.advanceTimersByTimeAsync(500); });
  const scroller = container.querySelector(".stream-scroll") as HTMLElement;
  Object.defineProperty(scroller, "scrollHeight", { value: 5000, configurable: true });
  Object.defineProperty(scroller, "clientHeight", { value: 600, configurable: true });
  scroller.scrollTop = 1000;
  fireEvent.wheel(scroller);
  fireEvent.scroll(scroller);
  const scroll = vi.mocked(scroller.scrollTo);
  scroll.mockClear();
  release = beginLayoutResize();
  scroller.scrollTop = 4400;
  fireEvent.scroll(scroller);
  scroller.scrollTop = 1000;
  act(() => release!());
  await act(async () => { await vi.advanceTimersByTimeAsync(32); });
  expect(scroll).not.toHaveBeenCalled();
  expect(scroller).toHaveAttribute("data-terminal-pinned", "false");
});
