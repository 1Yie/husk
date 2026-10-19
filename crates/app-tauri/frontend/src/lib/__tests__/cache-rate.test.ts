import { describe, expect, test } from "vitest";
import { formatCacheRate } from "../cache-rate";

describe("formatCacheRate", () => {
  test("exact ratios are not pulled down by float artefacts", () => {
    expect(formatCacheRate(29, 100)).toBe("29%");
    expect(formatCacheRate(57, 100)).toBe("57%");
    expect(formatCacheRate(7, 100)).toBe("7%");
  });

  test("truncates instead of rounding, so near-full never reads 100%", () => {
    expect(formatCacheRate(9999, 10000)).toBe("99.99%");
    expect(formatCacheRate(99999, 100000)).toBe("99.99%");
    expect(formatCacheRate(1, 3)).toBe("33.33%");
    expect(formatCacheRate(2, 3)).toBe("66.66%");
  });

  test("clamps and handles empty prompts", () => {
    expect(formatCacheRate(0, 0)).toBeNull();
    expect(formatCacheRate(5, 0)).toBeNull();
    expect(formatCacheRate(150, 100)).toBe("100%");
    expect(formatCacheRate(-3, 100)).toBe("0%");
  });
});
