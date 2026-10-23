// The capsule row above the composer: one capsule speaks, two put the second
// to sleep, three keep the first two open and fold only the third.

import { screen } from "@testing-library/react";
import { describe, expect, test } from "vitest";
import { PillIsland, pillCollapsed, type IslandPill } from "@/features/chat/components/pill-island";
import { renderWithProviders } from "@/test/render";

const pill = (label: string): IslandPill => ({
  key: label,
  label,
  tooltip: label,
  icon: <span />,
  text: `${label} text`,
});

const compact = (label: string) =>
  screen.getByRole("button", { name: label }).getAttribute("data-compact");

describe("pillCollapsed", () => {
  test("only the last of two or more folds", () => {
    expect(pillCollapsed(0, 1)).toBe(false);
    expect([0, 1].map((i) => pillCollapsed(i, 2))).toEqual([false, true]);
    expect([0, 1, 2].map((i) => pillCollapsed(i, 3))).toEqual([false, false, true]);
  });
});

describe("PillIsland", () => {
  test("renders nothing without capsules", () => {
    const { container } = renderWithProviders(<PillIsland pills={[]} />);
    expect(container.querySelector("[data-pill-island]")).toBeNull();
  });

  test("a lone capsule speaks in full", () => {
    renderWithProviders(<PillIsland pills={[pill("a")]} />);
    expect(compact("a")).toBeNull();
  });

  test("two capsules: the second folds", () => {
    renderWithProviders(<PillIsland pills={[pill("a"), pill("b")]} />);
    expect(compact("a")).toBeNull();
    expect(compact("b")).toBe("true");
  });

  test("three capsules: the first two speak, the third folds", () => {
    renderWithProviders(<PillIsland pills={[pill("a"), pill("b"), pill("c")]} />);
    expect(compact("a")).toBeNull();
    expect(compact("b")).toBeNull();
    expect(compact("c")).toBe("true");
  });

  test("the capsules never take a text selection", () => {
    const { container } = renderWithProviders(<PillIsland pills={[pill("a")]} />);
    expect(container.querySelector("[data-pill-island]")).toHaveClass("select-none");
  });

  test("spacing belongs to the row, not to any one capsule", () => {
    const { container } = renderWithProviders(<PillIsland pills={[pill("a"), pill("b"), pill("c")]} />);
    expect(container.querySelector("[data-pill-island]")).toHaveClass("gap-2");
    for (const b of screen.getAllByRole("button")) expect(b.className).not.toMatch(/\bm[lr]-/);
  });
});
