import { useState } from "react";
import { act, createEvent, fireEvent, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { renderWithProviders as render } from "@/test/render";
import { TestResizeObserver } from "@/test/dom";
import { TerminalTabStrip } from "../terminal-tab-strip";
import { TerminalTab } from "../terminal-tab";

let width: number;
const close = vi.fn();
const reordered = vi.fn();
beforeEach(() => {
  width = 240;
  const offsets = new WeakMap<Element, number>();
  Object.defineProperties(HTMLElement.prototype, {
    clientWidth: { configurable: true, get() { return this.matches(".terminal-panel__tabs") ? width : 800; } },
    scrollWidth: { configurable: true, get() { return this.matches(".terminal-panel__tabs") ? this.children.length * 100 : 800; } },
    scrollLeft: { configurable: true,
      get() { return offsets.get(this) ?? 0; },
      set(value: number) { offsets.set(this, Math.max(0, Math.min(this.scrollWidth - this.clientWidth, value))); },
    },
  });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    const viewport = this.closest(".terminal-panel__tabs") as HTMLElement | null;
    const tab = this.closest("[data-terminal-tab-key]");
    const index = viewport && tab ? [...viewport.children].indexOf(tab) : 0;
    return new DOMRect(10 + (tab ? index * 100 - (viewport?.scrollLeft ?? 0) : 0), 20, tab ? 100 : width, 24);
  });
});

function Fixture({ initial = 4, open = true }: { initial?: number; open?: boolean }) {
  const [keys, setKeys] = useState([1, 2, 3, 4]);
  const [active, setActive] = useState(initial);
  const [reveal, setReveal] = useState<{ key: number }>();
  const reorder = (key: number, before: number | null) => {
    reordered(key, before);
    setKeys((old) => {
      const next = old.filter((value) => value !== key);
      next.splice(before === null ? next.length : next.indexOf(before), 0, key);
      return next;
    });
    setReveal({ key });
  };
  return <TerminalTabStrip active={active} open={open} order={keys.join(",")} reveal={reveal} onReorder={reorder}>
    {keys.map((key, index) => <TerminalTab key={key} id="fixture" groupKey={key} label={`tab-${key}`}
      selected={active === key} open={open} panes={1}
      previous={keys[(index + 3) % 4]} next={keys[(index + 1) % 4]} first={keys[0]} last={keys[3]}
      canCreate canColumns canRows canCloseOthers
      canMoveLeft={index > 0} canMoveRight={index < 3}
      onSelect={setActive} onClose={close} onNew={() => {}} onSplit={() => {}} onRename={() => {}}
      onCloseOthers={() => {}} onCloseAll={() => {}}
      onMove={(key, direction) => {
        if (index + direction < 0 || index + direction >= keys.length) return;
        reorder(key, direction === -1 ? keys[index - 1] : keys[index + 2] ?? null);
      }} />)}
  </TerminalTabStrip>;
}
const viewport = () => screen.getByRole("tablist");
const tab = (key: number) => screen.getByRole("tab", { name: `tab-${key}` });
const order = () => screen.getAllByRole("tab").map((node) => node.textContent);
function pointer(target: Element | Window, type: "pointerDown" | "pointerMove" | "pointerUp" | "pointerCancel", x: number, y = 32, extra = {}) {
  const event = createEvent[type](target, { button: 0, clientX: x, clientY: y, bubbles: true, cancelable: true });
  Object.assign(event, { pointerId: 1, isPrimary: true, ...extra });
  fireEvent(target, event);
}

test("reveals selection and keyboard targets without scrolling ancestor content", () => {
  render(<Fixture />);
  expect(viewport().scrollLeft).toBe(160);
  fireEvent.keyDown(tab(4), { key: "Home" });
  expect(tab(1)).toHaveFocus();
  expect(tab(1)).toHaveAttribute("aria-selected", "true");
  expect(viewport().scrollLeft).toBe(0);
  fireEvent.keyDown(tab(1), { key: "End" });
  expect(tab(4)).toHaveFocus();
  expect(viewport().scrollLeft).toBe(160);
  expect(HTMLElement.prototype.scrollIntoView).not.toHaveBeenCalled();
});

test("maps dominant wheel axis and delta units, preserves pinch zoom, and supplies navigation", () => {
  render(<Fixture initial={1} />);
  const wheel = (options: WheelEventInit) => {
    const event = new WheelEvent("wheel", { cancelable: true, bubbles: true, ...options });
    fireEvent(viewport(), event);
    return event.defaultPrevented;
  };
  expect(wheel({ deltaY: 40 })).toBe(true);
  expect(viewport().scrollLeft).toBe(40);
  expect(wheel({ deltaX: 30, deltaY: 2 })).toBe(true);
  expect(viewport().scrollLeft).toBe(70);
  wheel({ deltaY: -2, deltaMode: 1 });
  expect(viewport().scrollLeft).toBe(38);
  expect(wheel({ deltaY: 100, ctrlKey: true })).toBe(false);
  expect(viewport().scrollLeft).toBe(38);
  fireEvent.click(screen.getByRole("button", { name: "向右滚动终端标签" }));
  expect(viewport().scrollLeft).toBe(160);
  expect(screen.getByRole("button", { name: "向右滚动终端标签" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "向左滚动终端标签" }));
  expect(viewport().scrollLeft).toBe(0);
  wheel({ deltaY: 1, deltaMode: 2 });
  expect(viewport().scrollLeft).toBe(160);
});

test("does not intercept wheel events or show navigation when every tab fits", () => {
  width = 500;
  render(<Fixture />);
  expect(screen.queryByRole("group", { name: "滚动终端标签" })).toBeNull();
  const wheel = new WheelEvent("wheel", { deltaY: 40, cancelable: true, bubbles: true });
  fireEvent(viewport(), wheel);
  expect(wheel.defaultPrevented).toBe(false);
  expect(viewport().scrollLeft).toBe(0);
  width = 240;
  act(() => TestResizeObserver.resize(viewport()));
  expect(screen.getByRole("group", { name: "滚动终端标签" })).toBeInTheDocument();
  expect(viewport().scrollLeft).toBe(160);
});

test("sorts after threshold, preserves selection, and suppresses release clicks on both tab and close", () => {
  render(<Fixture initial={1} />);
  pointer(tab(1), "pointerDown", 30);
  pointer(window, "pointerMove", 33);
  expect(document.querySelector("[data-dragging]")).toBeNull();
  pointer(window, "pointerMove", 190);
  expect(tab(1).parentElement).toHaveAttribute("data-dragging", "true");
  pointer(window, "pointerUp", 190);
  expect(order()).toEqual(["tab-2", "tab-1", "tab-3", "tab-4"]);
  expect(tab(1)).toHaveAttribute("aria-selected", "true");
  fireEvent.click(tab(2), { detail: 1 });
  fireEvent.click(screen.getByRole("button", { name: "关闭终端 tab-2" }), { detail: 1 });
  expect(close).not.toHaveBeenCalled();
  expect(tab(1)).toHaveAttribute("aria-selected", "true");
  const context = createEvent.contextMenu(tab(1));
  fireEvent(tab(1), context);
  expect(screen.queryByRole("menu")).toBeNull();
  const button = screen.getByRole("button", { name: "关闭终端 tab-2" });
  pointer(button, "pointerDown", 100);
  fireEvent.click(button, { detail: 1 });
  expect(close).toHaveBeenCalledWith(2);
});

test.each(["pointerCancel", "captureLost", "blur", "outside", "hidden", "unmount"] as const)("cleans drag markers and cancels reorder on %s", (reason) => {
  const view = render(<Fixture initial={1} />);
  pointer(tab(1), "pointerDown", 30);
  pointer(window, "pointerMove", 190);
  expect(document.querySelector("[data-dragging]")).not.toBeNull();
  if (reason === "pointerCancel") pointer(window, "pointerCancel", 190);
  else if (reason === "captureLost") {
    const event = new Event("lostpointercapture", { bubbles: true });
    Object.assign(event, { pointerId: 1 });
    fireEvent(viewport(), event);
  }
  else if (reason === "blur") fireEvent(window, new Event("blur"));
  else if (reason === "hidden") view.rerender(<Fixture initial={1} open={false} />);
  else if (reason === "unmount") view.unmount();
  else pointer(window, "pointerUp", 190, -100);
  expect(document.querySelector("[data-dragging]")).toBeNull();
  pointer(window, "pointerUp", 190);
  expect(reordered).not.toHaveBeenCalled();
});

test("a secondary touch cannot clear release-click protection or cancel the primary drag", () => {
  render(<Fixture initial={1} />);
  pointer(tab(1), "pointerDown", 30);
  pointer(window, "pointerMove", 190);
  pointer(tab(2), "pointerDown", 150, 32, { pointerId: 2, isPrimary: false });
  pointer(window, "pointerCancel", 150, 32, { pointerId: 2 });
  expect(tab(1).parentElement).toHaveAttribute("data-dragging", "true");
  pointer(window, "pointerUp", 190);
  expect(order()).toEqual(["tab-2", "tab-1", "tab-3", "tab-4"]);
  fireEvent.click(screen.getByRole("button", { name: "关闭终端 tab-2" }), { detail: 1 });
  expect(close).not.toHaveBeenCalled();
});

test("edge scrolling continues without pointer moves and stops on cancel", () => {
  vi.useFakeTimers();
  render(<Fixture initial={1} />);
  pointer(tab(1), "pointerDown", 30);
  pointer(window, "pointerMove", 245);
  act(() => vi.advanceTimersByTime(250));
  expect(viewport().scrollLeft).toBeGreaterThan(100);
  pointer(window, "pointerCancel", 245);
  viewport().scrollLeft = 0;
  act(() => vi.advanceTimersByTime(250));
  expect(viewport().scrollLeft).toBe(0);
});

test("does not drag close controls, secondary buttons, nonprimary pointers or rename inputs", () => {
  render(<Fixture initial={1} />);
  for (const target of [screen.getByRole("button", { name: "关闭终端 tab-1" }), tab(1)]) {
    pointer(target, "pointerDown", 30, 32, target === tab(1) ? { isPrimary: false } : {});
    pointer(window, "pointerMove", 190);
    pointer(window, "pointerUp", 190);
  }
  fireEvent.pointerDown(tab(1), { button: 2, clientX: 30 });
  pointer(window, "pointerMove", 190);
  pointer(window, "pointerUp", 190);
  expect(reordered).not.toHaveBeenCalled();
  fireEvent.contextMenu(tab(1));
  fireEvent.click(screen.getByRole("menuitem", { name: "重命名" }));
  const input = screen.getByRole("textbox", { name: "终端标签名称" });
  pointer(input, "pointerDown", 30);
  pointer(window, "pointerMove", 190);
  pointer(window, "pointerUp", 190);
  expect(reordered).not.toHaveBeenCalled();
});

test("keyboard/menu alternatives reorder and subsequent selection overrides the old reveal request", () => {
  render(<Fixture initial={1} />);
  fireEvent.keyDown(tab(1), { key: "ArrowRight", altKey: true });
  expect(order()).toEqual(["tab-2", "tab-1", "tab-3", "tab-4"]);
  fireEvent.contextMenu(tab(1));
  fireEvent.click(screen.getByRole("menuitem", { name: "向左移动标签" }));
  expect(order()).toEqual(["tab-1", "tab-2", "tab-3", "tab-4"]);
  fireEvent.click(tab(4));
  expect(viewport().scrollLeft).toBe(160);
  width = 120;
  act(() => TestResizeObserver.resize(viewport()));
  expect(viewport().scrollLeft).toBe(280);
});
