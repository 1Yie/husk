// The toolbar's tiers are a pure function of its own width, so the thresholds
// are testable without layout — jsdom never measures anything.

import { describe, expect, test } from "vitest";
import {
  TOOLBAR_COMPACT_MIN,
  TOOLBAR_FULL_MIN,
  toolbarDensity,
} from "@/features/chat/components/composer-bar/toolbar-density";

describe("toolbar density tiers", () => {
  test("a wide row keeps the labels", () => {
    expect(toolbarDensity(TOOLBAR_FULL_MIN)).toBe("full");
    expect(toolbarDensity(900)).toBe("full");
  });

  test("a mid row drops the labels but keeps one button per control", () => {
    expect(toolbarDensity(TOOLBAR_FULL_MIN - 1)).toBe("compact");
    expect(toolbarDensity(TOOLBAR_COMPACT_MIN)).toBe("compact");
    expect(toolbarDensity(480)).toBe("compact");
  });

  test("a narrow row folds the mode controls into one button", () => {
    expect(toolbarDensity(TOOLBAR_COMPACT_MIN - 1)).toBe("tight");
    expect(toolbarDensity(360)).toBe("tight");
    expect(toolbarDensity(0)).toBe("tight");
  });
});
