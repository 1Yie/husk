import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { GlobalTooltip, TooltipProvider, TooltipSimple } from "../tooltip";

const SHOW_DELAY_MS = 250;
/** Mirrors HOVER_GRACE_MS in tooltip.tsx: how long the pointer gets to cross the
 *  gap between a trigger and a scrollable bubble. */
const GRACE_MS = 120;

/** A session body / stack-trace shaped title: thousands of characters, which is
 *  the case that used to paint a window-sized popup. Normalised form (single
 *  spaces, no trailing space) so it can be matched verbatim by `getByText`. */
const LONG_TITLE = Array.from({ length: 400 }, () => "session body").join(" ");

function renderWithAnchor(title: string) {
  const view = render(
    <>
      <button type="button" title={title}>
        anchor
      </button>
      <GlobalTooltip />
    </>,
  );
  return { ...view, anchor: screen.getByRole("button", { name: "anchor" }) };
}

/** Hover the anchor and let the tooltip's show delay elapse. */
function showBubble(anchor: HTMLElement) {
  fireEvent.mouseOver(anchor);
  act(() => {
    vi.advanceTimersByTime(SHOW_DELAY_MS + 50);
  });
}

describe("GlobalTooltip native-title bubble", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  test("a long title renders a height-capped, scrollable bubble", () => {
    const { anchor } = renderWithAnchor(LONG_TITLE);
    showBubble(anchor);

    const bubble = screen.getByText(LONG_TITLE);
    expect(bubble.className).toContain("max-h-[min(20rem,calc(100vh-16px))]");
    expect(bubble.className).toContain("overflow-y-auto");
    expect(bubble.className).toContain("overscroll-contain");
    // Wrapping stays what it always was; only the height is new.
    expect(bubble.className).toContain("break-words");
    expect(bubble.className).toContain("max-w-xs");
  });

  test("an overflowing bubble survives the pointer crossing into it and closes after leaving both", () => {
    // jsdom has no layout engine: declare the overflow the capped box would have.
    vi.spyOn(Element.prototype, "scrollHeight", "get").mockReturnValue(1200);
    vi.spyOn(Element.prototype, "clientHeight", "get").mockReturnValue(320);

    const { anchor } = renderWithAnchor(LONG_TITLE);
    showBubble(anchor);

    const bubble = screen.getByText(LONG_TITLE);
    // Only an overflowing bubble may take the pointer, so it can be scrolled.
    expect(bubble.className).toContain("pointer-events-auto");
    expect(bubble.className).toContain("select-text");
    expect(bubble.className).not.toContain("pointer-events-none");

    // The bubble floats 6px away, so the pointer is briefly over neither.
    fireEvent.mouseOut(anchor, { relatedTarget: document.body });
    act(() => {
      vi.advanceTimersByTime(40);
    });
    expect(screen.getByText(LONG_TITLE)).toBeInTheDocument();

    fireEvent.mouseOver(bubble);
    act(() => {
      vi.advanceTimersByTime(GRACE_MS + 200);
    });
    expect(screen.getByText(LONG_TITLE)).toBeInTheDocument();

    fireEvent.mouseOut(bubble, { relatedTarget: document.body });
    act(() => {
      vi.advanceTimersByTime(GRACE_MS - 10);
    });
    expect(screen.getByText(LONG_TITLE)).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(40);
    });
    expect(screen.queryByText(LONG_TITLE)).toBeNull();
  });

  test("a short title stays click-through and closes the moment the pointer leaves", () => {
    const { anchor } = renderWithAnchor("short hint");
    showBubble(anchor);

    const bubble = screen.getByText("short hint");
    expect(bubble.className).toContain("pointer-events-none");
    expect(bubble.className).not.toContain("pointer-events-auto");
    expect(bubble.className).not.toContain("select-text");
    // Still bounded, so a title that grows later cannot cover the window.
    expect(bubble.className).toContain("max-h-[min(20rem,calc(100vh-16px))]");

    fireEvent.mouseOut(anchor, { relatedTarget: document.body });
    expect(screen.queryByText("short hint")).toBeNull();
  });

  test("scrolling inside the bubble keeps it open, scrolling the page does not", () => {
    vi.spyOn(Element.prototype, "scrollHeight", "get").mockReturnValue(1200);
    vi.spyOn(Element.prototype, "clientHeight", "get").mockReturnValue(320);

    const { anchor } = renderWithAnchor(LONG_TITLE);
    showBubble(anchor);
    const bubble = screen.getByText(LONG_TITLE);

    // The wheel spends itself on the bubble's own text, not on the page behind.
    fireEvent.scroll(bubble);
    expect(screen.getByText(LONG_TITLE)).toBeInTheDocument();

    fireEvent.scroll(window);
    expect(screen.queryByText(LONG_TITLE)).toBeNull();
  });

  test("a hidden bubble leaves the title intercepted, and hovering again shows the same text", () => {
    const { anchor } = renderWithAnchor("short hint");
    showBubble(anchor);
    // Taken off the element so the browser never paints its own bubble.
    expect(anchor.hasAttribute("title")).toBe(false);
    expect(anchor.getAttribute("data-tooltip")).toBe("short hint");

    // Clicking the trigger dismisses the bubble, but the pointer stays resting
    // on the button: a title handed back here would pop the native tooltip.
    fireEvent.pointerDown(anchor);
    expect(screen.queryByText("short hint")).toBeNull();
    expect(anchor.hasAttribute("title")).toBe(false);

    // Hovering the same element again brings the bubble back, unchanged.
    showBubble(anchor);
    expect(screen.getByText("short hint")).toBeInTheDocument();
    expect(anchor.hasAttribute("title")).toBe(false);
    expect(anchor.getAttribute("data-tooltip")).toBe("short hint");
  });

  test.each(["open", "closed", "checked", "unchecked"])("a Radix %s control never leaks its native title", (state) => {
    render(<>
      <button data-state={state} title="菜单提示">menu control</button>
      <GlobalTooltip />
    </>);
    const anchor = screen.getByRole("button", { name: "menu control" });
    showBubble(anchor);
    expect(anchor).not.toHaveAttribute("title");
    expect(screen.getByText("菜单提示")).toBeInTheDocument();
  });

  test("an explicit shared Tooltip suppresses native title without a second fallback bubble", () => {
    render(<TooltipProvider>
      <TooltipSimple content="统一提示">
        <button title="旧原生提示">explicit tooltip</button>
      </TooltipSimple>
      <GlobalTooltip />
    </TooltipProvider>);
    const anchor = screen.getByRole("button", { name: "explicit tooltip" });
    showBubble(anchor);
    expect(anchor).not.toHaveAttribute("title");
    expect(anchor).toHaveAttribute("data-husk-tooltip-trigger");
    expect(screen.queryByText("旧原生提示")).not.toBeInTheDocument();
  });
});
