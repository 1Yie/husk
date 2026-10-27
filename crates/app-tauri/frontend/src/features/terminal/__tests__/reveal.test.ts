import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { afterTerminalReveal } from "../reveal";

let frames: Map<number, FrameRequestCallback>;
let nextId: number;
beforeEach(() => {
  frames = new Map();
  nextId = 0;
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    frames.set(++nextId, callback);
    return nextId;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
});
afterEach(() => vi.unstubAllGlobals());
function frame() {
  const callbacks = [...frames.values()];
  frames.clear();
  for (const callback of callbacks) callback(performance.now());
}

test("initializes only after the expanded surface has had a paint opportunity", () => {
  const panel = document.createElement("div");
  let height = 0;
  panel.getBoundingClientRect = () => new DOMRect(0, 280 - height, 800, height);
  const activate = vi.fn();
  afterTerminalReveal(panel, () => 280, activate);
  for (height of [0, 10, 100, 250, 279]) frame();
  expect(activate).not.toHaveBeenCalled();
  height = 280;
  frame();
  expect(activate).not.toHaveBeenCalled();
  frame();
  expect(activate).toHaveBeenCalledOnce();
  frame();
  expect(activate).toHaveBeenCalledOnce();
});

test("initially open and reduced-motion panels do not need a transitionend event", () => {
  const panel = document.createElement("div");
  panel.getBoundingClientRect = () => new DOMRect(0, 0, 800, 280);
  const activate = vi.fn();
  afterTerminalReveal(panel, () => 280, activate);
  frame();
  expect(activate).not.toHaveBeenCalled();
  frame();
  expect(activate).toHaveBeenCalledOnce();
});

test("a cancelled reveal never starts the renderer", () => {
  const panel = document.createElement("div");
  panel.getBoundingClientRect = () => new DOMRect(0, 0, 800, 280);
  const activate = vi.fn();
  const cancel = afterTerminalReveal(panel, () => 280, activate);
  frame();
  cancel();
  frame();
  expect(activate).not.toHaveBeenCalled();
});

test("a changed target must finish revealing before activation", () => {
  const panel = document.createElement("div");
  let height = 280;
  let target = 280;
  panel.getBoundingClientRect = () => new DOMRect(0, 280 - height, 800, height);
  const activate = vi.fn();
  afterTerminalReveal(panel, () => target, activate);
  frame();
  target = 400;
  frame();
  expect(activate).not.toHaveBeenCalled();
  height = 400;
  frame();
  expect(activate).not.toHaveBeenCalled();
  frame();
  expect(activate).toHaveBeenCalledOnce();
});

test("cold initialization waits for a concurrent window width drag to settle", () => {
  const panel = document.createElement("div");
  let width = 1280;
  panel.getBoundingClientRect = () => new DOMRect(0, 0, width, 280);
  const activate = vi.fn();
  afterTerminalReveal(panel, () => 280, activate);
  frame();
  for (width of [1200, 1100, 1000]) frame();
  expect(activate).not.toHaveBeenCalled();
  frame();
  expect(activate).not.toHaveBeenCalled();
  frame();
  expect(activate).toHaveBeenCalledOnce();
});

test("dragging pauses settlement until the final size is committed", () => {
  const panel = document.createElement("div");
  panel.getBoundingClientRect = () => new DOMRect(0, 0, 800, 280);
  let target: number | null = null;
  const settle = vi.fn();
  afterTerminalReveal(panel, () => target, settle);
  frame();
  frame();
  expect(settle).not.toHaveBeenCalled();
  target = 280;
  frame();
  expect(settle).not.toHaveBeenCalled();
  frame();
  expect(settle).toHaveBeenCalledOnce();
});

test("closing waits for zero real height, not a hidden translated shell", () => {
  const panel = document.createElement("div");
  let height = 280;
  panel.getBoundingClientRect = () => new DOMRect(0, 0, 800, height);
  const settle = vi.fn();
  afterTerminalReveal(panel, () => 0, settle);
  for (height of [280, 240, 80, 1]) frame();
  expect(settle).not.toHaveBeenCalled();
  height = 0;
  frame();
  frame();
  expect(settle).toHaveBeenCalledOnce();
});
