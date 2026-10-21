import { expect, test } from "vitest";
import {
  highlightCodeToHtml,
  highlightCodeToLines,
  peekHighlightedLines,
  scheduleHighlightLines,
} from "@/lib/syntax-highlight";

test("peek never tokenizes; it only reads what was highlighted before", () => {
  const code = "fn peeked() -> u8 { 1 }";
  expect(peekHighlightedLines(code, "rust")).toBeUndefined();
  const lines = highlightCodeToLines(code, "rust");
  expect(peekHighlightedLines(code, "rust")).toBe(lines);
});

test("queued highlights land in order and a cancelled one never calls back", async () => {
  const seen: string[] = [];
  scheduleHighlightLines("const a = 1;", "typescript", () => seen.push("a"));
  const cancel = scheduleHighlightLines("const b = 2;", "typescript", () => seen.push("b"));
  const done = new Promise<void>((resolve) =>
    scheduleHighlightLines("const c = 3;", "typescript", (lines) => {
      seen.push("c");
      expect(lines).toEqual(highlightCodeToLines("const c = 3;", "typescript"));
      resolve();
    }),
  );
  cancel();
  await done;
  expect(seen).toEqual(["a", "c"]);
});

test("highlighted lines and the joined HTML describe the same block", () => {
  const code = ['const value = "x";', "// note", ""].join("\n");
  const lines = highlightCodeToLines(code, "typescript");
  expect(lines).toHaveLength(3);
  expect(lines.join("\n")).toBe(highlightCodeToHtml(code, "typescript"));
});

test("a language with no grammar still escapes every line", () => {
  expect(highlightCodeToLines("<tag>", "text")).toEqual(["&lt;tag&gt;"]);
});
