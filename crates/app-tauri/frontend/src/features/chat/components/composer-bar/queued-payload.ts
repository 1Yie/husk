// A submitted prompt travels as one text payload: the typed text, then one
// marker block per attachment. The chat stream strips the same markers out of
// user bubbles. This module is the one place that knows the layout — submit
// builds it, and a queued row's 编辑 takes it apart again.

import type { Attachment } from "@/lib/agent-ipc/index";

/** An attachment recovered from a payload. `kind` is the marker it came from;
 *  the content is never read back from the payload — it is re-read by `path`. */
export interface QueuedAttachmentRef {
  path: string;
  kind: "text" | "image" | "binary";
}

export interface SplitQueuedPayload {
  text: string;
  paths: QueuedAttachmentRef[];
}

/** One whole block per match, each opening on a paragraph break. A text block
 *  runs to its closing fence; the lazy body stops at the first fence that
 *  closes it — `buildPayload` writes the fence as the block's last line, so the
 *  body's own text never reads as a block of its own. */
const BLOCK =
  /\n\n(?:<attached-file path="([^"]+)">\n```\n[\s\S]*?\n```(?=\n\n|$)|<attached-image path="([^"]+)"(?: name="[^"]*")?\/>|\[attached binary: (.+?) — binary file, not inlined\])/g;

/** The prompt a composer submission sends: trimmed text, then one block per attachment. */
export function buildPayload(text: string, attachments: Attachment[]): string {
  const blocks = attachments
    .map((a) =>
      a.kind === "text" && a.content != null
        ? `\n\n<attached-file path="${a.path}">\n\`\`\`\n${a.content}${a.truncated ? "\n… (truncated)" : ""}\n\`\`\``
        : a.kind === "image"
          ? `\n\n<attached-image path="${a.path}" name="${a.name.replace(/"/g, "")}"/>`
          : `\n\n[attached binary: ${a.path} — binary file, not inlined]`,
    )
    .join("");
  return `${text.trim()}${blocks}`;
}

/** Inverse of `buildPayload`: the typed text plus the files its blocks name.
 *  A payload without markers is all text. */
export function splitQueuedPayload(payload: string): SplitQueuedPayload {
  const matches = Array.from(payload.matchAll(BLOCK));
  const text = payload.slice(0, matches[0]?.index ?? payload.length);
  const paths: QueuedAttachmentRef[] = matches.map((m) => {
    const [, file, image, binary] = m;
    if (file !== undefined) return { path: file, kind: "text" };
    if (image !== undefined) return { path: image, kind: "image" };
    return { path: binary ?? "", kind: "binary" };
  });
  return { text, paths };
}

/** `list` with `item` spliced in at `index`; an index past the end appends. */
export function insertAt(list: string[], index: number, item: string): string[] {
  return [...list.slice(0, index), item, ...list.slice(index)];
}

/** What a queued row previews: the first line of the typed text, how many
 *  lines follow it, and the files its blocks name (by base name). */
export function queuedPreview(payload: string) {
  const { text, paths } = splitQueuedPayload(payload);
  const lines = text.trim().split("\n");
  return {
    head: lines[0],
    moreLines: lines.length - 1,
    paths: paths.map((p) => ({ ...p, name: p.path.split("/").pop() ?? p.path })),
  };
}
