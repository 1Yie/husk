import { render } from "@testing-library/react";
import { expect, test } from "vitest";
import { WorkspaceFileIcon, workspaceFileIcon } from "../file-icon";

test.each([
  ["main.js", "_javascript"],
  ["main.mjs", "_javascript"],
  ["MAIN.TS", "_typescript"],
  ["data.json", "_json"],
  ["notes.md", "_markdown"],
  ["README.md", "_info"],
  ["main.py", "_python"],
  ["main.rs", "_rust"],
  ["image.png", "_image"],
])("%s uses its VS Code file-type icon", (name, expected) => {
  const icon = workspaceFileIcon(name);
  expect(icon.dark.name).toBe(expected);
  expect(icon.light.glyph).toBe(icon.dark.glyph);
  expect(icon.dark.glyph).not.toBe("");
});

test("paths resolve by basename and unknown names fall back safely", () => {
  expect(workspaceFileIcon("/workspace/src/main.js")).toEqual(workspaceFileIcon("main.js"));
  expect(workspaceFileIcon("C:\\workspace\\main.ts")).toEqual(workspaceFileIcon("main.ts"));
  expect(workspaceFileIcon("__proto__").dark.name).toBe("_default");
  expect(workspaceFileIcon("constructor").dark.name).toBe("_default");
});

test("file icons are transparent and have both theme variants without changing the background", () => {
  const view = render(<WorkspaceFileIcon name="main.js" />);
  const icon = view.container.querySelector<HTMLElement>("[data-file-icon]");
  expect(icon).toHaveAttribute("data-file-icon", "javascript");
  expect(icon).toHaveAttribute("aria-hidden", "true");
  expect(icon?.style.background).toBe("");
  expect(icon?.style.getPropertyValue("--file-icon-light")).not.toBe(
    icon?.style.getPropertyValue("--file-icon-dark"),
  );
  expect(icon?.querySelector(".workspace-file-icon__light")).not.toBeNull();
  expect(icon?.querySelector(".workspace-file-icon__dark")).not.toBeNull();
});
