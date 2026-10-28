import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";
import { ReasoningEffort } from "../reasoning-effort";
import { thinkingTone } from "@/lib/thinking";

describe("reasoning effort tier colour", () => {
  test("the tier word takes the tone of the stop it sits on", () => {
    const values = ["low", "medium", "max"];
    render(
      <ReasoningEffort
        stops={["Low", "Medium", "Max"]}
        levelTones={values.map(thinkingTone)}
        value={2}
        label="Claude Opus 5.5"
      />,
    );
    expect(screen.getByText("Max")).toHaveStyle({ color: "var(--husk-think-max, var(--husk-fg-muted))" });
    // The model name stays the caption colour — only the tier is toned.
    expect(screen.getByText("Claude Opus 5.5").getAttribute("style")).toBeNull();
  });

  test("without tones the tier keeps the dim caption colour", () => {
    render(<ReasoningEffort stops={["Low", "High"]} value={1} label="m" />);
    expect(screen.getByText("High").getAttribute("style")).toBeNull();
  });

  test("an unknown tier falls back to the muted ink", () => {
    expect(thinkingTone("turbo")).toBe("var(--husk-think-turbo, var(--husk-fg-muted))");
  });

  test("Fast keeps front sparkles during the reveal, then starts background points only after transitionend", () => {
    const onChange = vi.fn();
    const { container, rerender } = render(
      <ReasoningEffort stops={["Low", "High"]} value={1} accent="blue" onChange={onChange} />,
    );
    const path = container.querySelector("svg path")!.getAttribute("d");
    expect(container.querySelector("[data-fast-fill]")).toBeInTheDocument();
    expect(container.querySelector("[data-fast-points]")).not.toBeInTheDocument();
    rerender(<ReasoningEffort stops={["Low", "High"]} value={1} accent="purple" onChange={onChange} />);
    expect(screen.getByRole("slider")).toHaveAttribute("aria-valuenow", "1");
    expect(container.querySelector("svg path")).toHaveAttribute("d", path);
    expect(container.querySelector("[data-fast-front]")).toBeInTheDocument();
    expect(container.querySelector("[data-fast-points]")).not.toBeInTheDocument();
    expect(container.querySelector("[data-fast-ambient]")).not.toBeInTheDocument();
    const event = new Event("transitionend", { bubbles: true });
    Object.defineProperty(event, "propertyName", { value: "transform" });
    fireEvent(container.querySelector("[data-fast-fill]")!, event);
    expect(container.querySelector("[data-fast-front]")).not.toBeInTheDocument();
    expect(container.querySelector("[data-fast-points]")).toBeInTheDocument();
    expect(container.querySelector("[data-fast-background]")!.parentElement).toBe(screen.getByRole("slider"));
    expect(onChange).not.toHaveBeenCalled();
  });

  test("a restored Fast state skips the reveal's front sparkles", () => {
    const { container } = render(<ReasoningEffort accent="purple" />);
    expect(container.firstChild).toHaveAttribute("data-accent", "purple");
    expect(container.querySelector("[data-fast-front]")).not.toBeInTheDocument();
  });

  test("opening completes before points appear, meteors follow later, and disabling or unmounting stops them", () => {
    vi.useFakeTimers();
    try {
      const { container, rerender, unmount } = render(<ReasoningEffort accent="purple" effectsEnabled={false} />);
      act(() => vi.advanceTimersByTime(5000));
      expect(container.querySelector("[data-fast-points]")).not.toBeInTheDocument();
      expect(container.querySelector("[data-fast-ambient]")).not.toBeInTheDocument();
      rerender(<ReasoningEffort accent="purple" effectsEnabled />);
      expect(container.querySelector("[data-fast-points]")!.childElementCount).toBe(10);
      expect(container.querySelector("[data-fast-ambient]")).not.toBeInTheDocument();
      act(() => vi.advanceTimersByTime(2201));
      expect(container.querySelectorAll("[data-fast-ambient]")).toHaveLength(1);
      fireEvent.animationEnd(container.querySelector("[data-fast-ambient]")!);
      expect(container.querySelector("[data-fast-ambient]")).not.toBeInTheDocument();
      rerender(<ReasoningEffort accent="blue" />);
      act(() => vi.advanceTimersByTime(10000));
      expect(container.querySelector("[data-fast-points]")).not.toBeInTheDocument();
      expect(container.querySelector("[data-fast-ambient]")).not.toBeInTheDocument();
      unmount();
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });
});
