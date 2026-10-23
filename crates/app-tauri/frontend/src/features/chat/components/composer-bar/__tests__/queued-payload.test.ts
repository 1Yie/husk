import { describe, expect, test } from "vitest";
import {
  buildPayload,
  insertAt,
  queuedPreview,
  splitQueuedPayload,
} from "../queued-payload";
import type { Attachment } from "@/lib/agent-ipc/index";

const text: Attachment = { path: "/ws/.husk/a.ts", name: "a.ts", kind: "text", content: "export const x = 1;" };
const truncated: Attachment = { ...text, path: "/ws/.husk/big.log", name: "big.log", content: "tail", truncated: true };
const image: Attachment = { path: "/ws/.husk/shot.png", name: "shot.png", kind: "image" };
const binary: Attachment = { path: "/ws/.husk/data.bin", name: "data.bin", kind: "binary" };

describe("queued payload", () => {
  test("text alone is trimmed and has no markers", () => {
    expect(buildPayload("  hello  ", [])).toBe("hello");
    expect(splitQueuedPayload("hello")).toEqual({ text: "hello", paths: [] });
  });

  test("buildPayload output splits back into text and each attachment", () => {
    const payload = buildPayload("看看这些", [text, truncated, image, binary]);
    expect(splitQueuedPayload(payload)).toEqual({
      text: "看看这些",
      paths: [
        { path: "/ws/.husk/a.ts", kind: "text" },
        { path: "/ws/.husk/big.log", kind: "text" },
        { path: "/ws/.husk/shot.png", kind: "image" },
        { path: "/ws/.husk/data.bin", kind: "binary" },
      ],
    });
  });

  test("a file body that itself contains marker-like text stays inside its block", () => {
    const tricky: Attachment = {
      ...text,
      content: 'See <attached-image path="/x.png"/>\n\n[attached binary: /y — binary file, not inlined]',
    };
    const payload = buildPayload("hi", [tricky]);
    expect(splitQueuedPayload(payload).paths).toEqual([{ path: "/ws/.husk/a.ts", kind: "text" }]);
  });

  test("an attachment-only payload has empty text", () => {
    const payload = buildPayload("", [image]);
    expect(splitQueuedPayload(payload)).toEqual({
      text: "",
      paths: [{ path: "/ws/.husk/shot.png", kind: "image" }],
    });
  });

  test("a text body spanning lines keeps every line in the text", () => {
    const payload = buildPayload("第一行\n第二行", []);
    expect(splitQueuedPayload(payload).text).toBe("第一行\n第二行");
  });

  test("preview takes the first line, counts the rest, and names files by base name", () => {
    const preview = queuedPreview(buildPayload("标题\n细节一\n细节二", [text]));
    expect(preview).toEqual({
      head: "标题",
      moreLines: 2,
      paths: [{ path: "/ws/.husk/a.ts", kind: "text", name: "a.ts" }],
    });
  });

  test("insertAt restores an item at its index and clamps past the end", () => {
    expect(insertAt(["a", "c"], 1, "b")).toEqual(["a", "b", "c"]);
    expect(insertAt(["a"], 5, "z")).toEqual(["a", "z"]);
    expect(insertAt([], 0, "only")).toEqual(["only"]);
  });
});
