import { act, renderHook } from "@testing-library/react";
import { StrictMode } from "react";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { usePopperPlaced } from "../use-popper-placed";

let frames: Map<number, FrameRequestCallback>;
let nextFrame: number;

beforeEach(() => {
  frames = new Map();
  nextFrame = 0;
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    frames.set(++nextFrame, callback);
    return nextFrame;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
});

function frame() {
  act(() => {
    const pending = [...frames.values()];
    frames.clear();
    pending.forEach((callback) => callback(performance.now()));
  });
}

function content(transform: string, top = 100) {
  const wrapper = document.createElement("div");
  wrapper.setAttribute("data-radix-popper-content-wrapper", "");
  wrapper.style.transform = transform;
  const node = document.createElement("div");
  wrapper.append(node);
  vi.spyOn(wrapper, "getBoundingClientRect").mockReturnValue({ top } as DOMRect);
  return { wrapper, node };
}

describe("popper placement lifecycle", () => {
  test("reveals a positioned menu even when its top extends above the viewport", () => {
    const { result } = renderHook(usePopperPlaced);
    const { node } = content("translate(180px, -40px)", -40);
    act(() => result.current.attach(node));
    frame();
    expect(result.current.placed).toBe(false);
    frame();
    expect(result.current.placed).toBe(true);
  });

  test("waits for a real transform instead of treating a zero-height parked node as positioned", () => {
    const { result } = renderHook(usePopperPlaced);
    const { wrapper, node } = content("translate(0, -200%)", 0);
    act(() => result.current.attach(node));
    for (let i = 0; i < 6; i++) frame();
    expect(result.current.placed).toBe(false);
    act(() => { wrapper.style.transform = "translate(100px, 200px)"; });
    frame();
    frame();
    expect(result.current.placed).toBe(true);
  });

  test("resets on reattachment and ignores the previous node's pending measurement", () => {
    const { result } = renderHook(usePopperPlaced);
    const first = content("translate(100px, 200px)");
    const second = content("translate(0, -200%)", -200);
    act(() => result.current.attach(first.node));
    frame();
    frame();
    expect(result.current.placed).toBe(true);
    act(() => {
      result.current.attach(null);
      result.current.attach(second.node);
    });
    expect(result.current.placed).toBe(false);
    frame();
    expect(result.current.placed).toBe(false);
    act(() => result.current.attach(null));
    expect(frames.size).toBe(0);
  });

  test("cancels polling on unmount, including StrictMode effect cleanup", () => {
    const { result, unmount } = renderHook(usePopperPlaced, { wrapper: StrictMode });
    act(() => result.current.attach(content("translate(0, -200%)", -200).node));
    frame();
    expect(frames.size).toBeGreaterThan(0);
    unmount();
    expect(frames.size).toBe(0);
  });
});
