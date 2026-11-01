import { act, render } from "@testing-library/react";
import { useRef } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { TestResizeObserver } from "@/test/dom";
import { useSidebarGlass } from "./sidebar-glass";

const boundary = vi.hoisted(() => ({ invoke: vi.fn(), isTauri: vi.fn(() => true) }));
vi.mock("@tauri-apps/api/core", () => boundary);
vi.mock("@/lib/platform", () => ({ framedShell: true }));

let frames: FrameRequestCallback[];
let width: number;
let canvasContext: CanvasRenderingContext2D;
function Fixture() {
  const ref = useRef<HTMLDivElement>(null);
  useSidebarGlass(ref);
  return <div ref={ref} data-testid="shell"><div data-testid="navigation" style={{ width: 240 }} /><main style={{ backgroundColor: "white" }} /></div>;
}
async function frame() {
  await act(async () => {
    const callbacks = frames;
    frames = [];
    callbacks.forEach((callback) => callback(0));
    await Promise.resolve();
  });
}
beforeEach(() => {
  frames = [];
  width = 1120;
  boundary.invoke.mockReset().mockResolvedValue(true);
  boundary.isTauri.mockReturnValue(true);
  document.documentElement.removeAttribute("data-sidebar-glass");
  document.documentElement.style.setProperty("--husk-panel", "#f3f3f5");
  document.documentElement.style.setProperty("--husk-bg", "#ffffff");
  vi.stubGlobal("requestAnimationFrame", vi.fn((callback) => { frames.push(callback); return frames.length; }));
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(() => ({
    x: 0, y: 0, width, height: 600, top: 0, left: 0, bottom: 600, right: width,
    toJSON: () => ({}),
  }));
  canvasContext = {
    fillStyle: "",
    clearRect() {}, fillRect() {}, getImageData: () => ({ data: [243, 243, 245, 255] }),
  } as unknown as CanvasRenderingContext2D;
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(canvasContext);
});

it("keeps the solid surface until native success, and restores it after failure", async () => {
  let resolve!: (active: boolean) => void;
  boundary.invoke.mockImplementationOnce(() => new Promise<boolean>((done) => { resolve = done; }));
  render(<Fixture />);
  await frame();
  expect(canvasContext.fillStyle).toBe("#f3f3f5");
  expect(boundary.invoke.mock.calls[0][1].surface.background).toEqual([243 / 255, 243 / 255, 245 / 255]);
  expect(document.documentElement).not.toHaveAttribute("data-sidebar-glass");
  const count = boundary.invoke.mock.calls.length;
  TestResizeObserver.resize(document.querySelector("[data-testid=navigation]")!);
  await frame();
  expect(boundary.invoke).toHaveBeenCalledTimes(count);
  await act(async () => resolve(true));
  expect(document.documentElement).toHaveAttribute("data-sidebar-glass");
  const navigation = document.querySelector<HTMLElement>("[data-testid=navigation]")!;
  navigation.style.width = "0px";
  TestResizeObserver.resize(navigation);
  await frame();
  expect(boundary.invoke).toHaveBeenCalledTimes(count);
  expect(boundary.invoke.mock.calls[0][1].surface).toMatchObject({ x: 0, y: 0, width: 1120, height: 600 });
  expect(document.documentElement).toHaveAttribute("data-sidebar-glass");
  boundary.invoke.mockResolvedValueOnce(false);
  window.dispatchEvent(new Event("husk-sidebar-glass-change"));
  await frame();
  expect(document.documentElement).not.toHaveAttribute("data-sidebar-glass");
  const error = vi.spyOn(console, "error").mockImplementation(() => {});
  boundary.invoke.mockRejectedValueOnce(new Error("native unavailable"));
  window.dispatchEvent(new Event("husk-sidebar-glass-change"));
  await frame();
  expect(document.documentElement).not.toHaveAttribute("data-sidebar-glass");
  expect(error).toHaveBeenCalledOnce();
});

it("coalesces frames, retains only the latest in-flight geometry, and releases after completion", async () => {
  let resolve!: (active: boolean) => void;
  boundary.invoke.mockImplementationOnce(() => new Promise<boolean>((done) => { resolve = done; }));
  const view = render(<Fixture />);
  await frame();
  const shell = view.getByTestId("shell");
  for (width of [1000, 900, 780]) {
    TestResizeObserver.resize(shell);
    TestResizeObserver.resize(shell);
    expect(frames).toHaveLength(1);
    await frame();
  }
  expect(boundary.invoke).toHaveBeenCalledTimes(1);
  await act(async () => resolve(true));
  expect(boundary.invoke).toHaveBeenCalledTimes(2);
  expect(boundary.invoke.mock.calls[1][1].surface.width).toBe(780);
  expect(frames).toHaveLength(0);
  view.unmount();
  await act(async () => {});
  expect(boundary.invoke.mock.calls.at(-1)?.[1].surface).toBeNull();
  expect(document.documentElement).not.toHaveAttribute("data-sidebar-glass");
  expect(TestResizeObserver.instances.size).toBe(0);
  const count = boundary.invoke.mock.calls.length;
  window.dispatchEvent(new Event("husk-sidebar-glass-change"));
  expect(frames).toHaveLength(0);
  expect(boundary.invoke).toHaveBeenCalledTimes(count);
});

it("serializes remounts, replaces an obsolete release, and skips non-Tauri windows", async () => {
  let resolve!: (active: boolean) => void;
  boundary.invoke.mockImplementationOnce(() => new Promise<boolean>((done) => { resolve = done; }));
  const view = render(<Fixture />);
  await frame();
  view.unmount();
  expect(boundary.invoke).toHaveBeenCalledTimes(1);
  width = 780;
  const replacement = render(<Fixture />);
  await frame();
  expect(boundary.invoke).toHaveBeenCalledTimes(1);
  await act(async () => resolve(true));
  expect(document.documentElement).toHaveAttribute("data-sidebar-glass");
  expect(boundary.invoke.mock.calls[1][1].surface.width).toBe(780);
  expect(boundary.invoke.mock.calls[1][1].revision).toBeGreaterThan(boundary.invoke.mock.calls[0][1].revision);
  replacement.unmount();
  await act(async () => {});
  expect(boundary.invoke.mock.calls.at(-1)?.[1].surface).toBeNull();
  boundary.isTauri.mockReturnValue(false);
  render(<Fixture />);
  expect(frames).toHaveLength(0);
});
