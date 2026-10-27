import { expect, test } from "vitest";
import { shortcutLabel } from "../platform";

test("shortcuts are spelled per platform, never both at once", () => {
  expect(shortcutLabel(["mod", "shift", "j"], true)).toBe("⌘⇧J");
  expect(shortcutLabel(["mod", "shift", "j"], false)).toBe("Ctrl+Shift+J");
  expect(shortcutLabel(["mod", "c"], true)).toBe("⌘C");
  expect(shortcutLabel(["mod", "alt", "Enter"], false)).toBe("Ctrl+Alt+Enter");
});
