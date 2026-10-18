import { act, fireEvent, render } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";
import { WindowResizeEdges } from "@/components/window-resize-edges";
import { tauri } from "@/test/tauri";

describe("frameless window resize edges", () => {
  test("an edge owns its cursor and hands a left press to the window manager", async () => {
    const { container } = render(<WindowResizeEdges />);
    await act(async () => { await Promise.resolve(); });
    const zones = container.querySelectorAll<HTMLElement>("[aria-hidden]");
    expect(zones).toHaveLength(8);
    const east = Array.from(zones).find((z) => z.style.cursor === "ew-resize" && z.style.right === "0px")!;
    fireEvent.pointerDown(east, { button: 2 });
    expect(tauri.window.startResizeDragging).not.toHaveBeenCalled();
    fireEvent.pointerDown(east, { button: 0 });
    expect(tauri.window.startResizeDragging).toHaveBeenCalledWith("East");
  });

  test("a maximized window has no edges to drag", async () => {
    tauri.window.isMaximized.mockResolvedValueOnce(true);
    const { container } = render(<WindowResizeEdges />);
    await act(async () => { await Promise.resolve(); });
    expect(container.querySelectorAll("[aria-hidden]")).toHaveLength(0);
  });

  test("a live resize flags the root until the size settles", async () => {
    render(<WindowResizeEdges />);
    await act(async () => { window.dispatchEvent(new Event("resize")); });
    expect(document.documentElement.dataset.windowResizing).toBe("");
    await act(async () => { await new Promise((r) => setTimeout(r, 200)); });
    expect(document.documentElement.dataset.windowResizing).toBeUndefined();
  });
  test("the edge steps aside for a scrollbar beneath it, and comes back off it", async () => {
    const { container } = render(<WindowResizeEdges />);
    await act(async () => { await Promise.resolve(); });
    const zones = Array.from(container.querySelectorAll<HTMLElement>("[aria-hidden]"));
    // A scroll box flush with the right edge: 6px vertical scrollbar gutter.
    const box = document.createElement("div");
    document.body.appendChild(box);
    const w = window.innerWidth, h = window.innerHeight;
    Object.defineProperties(box, {
      offsetWidth: { value: 400 }, clientWidth: { value: 394 },
      offsetHeight: { value: h }, clientHeight: { value: h },
    });
    box.getBoundingClientRect = () => ({ left: w - 400, top: 0, right: w, bottom: h, width: 400, height: h, x: w - 400, y: 0, toJSON() {} });
    const stub = vi.fn((x: number) => (x >= w - 400 ? [box, document.body] : [document.body]));
    Object.defineProperty(document, "elementsFromPoint", { value: stub, configurable: true });
    fireEvent.pointerMove(document, { clientX: w - 3, clientY: 300, buttons: 0 });
    expect(zones.every((z) => z.style.pointerEvents === "none")).toBe(true);
    // Off the gutter (still inside the box) the edges own the pointer again.
    fireEvent.pointerMove(document, { clientX: w - 10, clientY: 300, buttons: 0 });
    expect(zones.every((z) => z.style.pointerEvents === "")).toBe(true);
    box.remove();
  });
});
