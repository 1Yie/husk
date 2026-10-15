import { describe, expect, test } from "vitest";
import { normalizeThinkingLevel, thinkingSuffix } from "../thinking";

describe("thinking level capabilities", () => {
  test("a supported selection and explicit off survive model reloads", () => {
    expect(normalizeThinkingLevel("high", ["off", "low", "high", "max"])).toBe("high");
    expect(normalizeThinkingLevel(" OFF ", ["off", "high"])).toBe("off");
  });
  test("unknown/restored tiers fall back to medium, first enabled tier, then off", () => {
    expect(normalizeThinkingLevel("xhigh", ["off", "medium", "high"])).toBe("medium");
    expect(normalizeThinkingLevel(null, ["off", "low", "max"])).toBe("low");
    expect(normalizeThinkingLevel(undefined, ["off"])).toBe("off");
    expect(normalizeThinkingLevel("max", [])).toBe("off");
  });
  test.each([
    [undefined, "high"], [{ high: null }, "high"], [{ high: "" }, "high"],
    [{ high: "high" }, "high"], [{ high: "max" }, "high→max"],
  ])("wire suffix reflects only an actual remapping (%j)", (map, expected) => {
    expect(thinkingSuffix("high", map)).toBe(expected);
  });
});
