// Composer token-highlight mirror — the transparent-text textarea
// overlays this layer, which paints @file//cmd/$skill chips aligned to
// the glyphs beneath.

import type { RefObject } from "react";

/** Chip background per token kind — painted by the composer mirror layer.
 * No horizontal padding: any extra width would desync the mirror from the
 * textarea glyphs underneath. */
const TOKEN_CHIP_CLS = {
  file: "rounded-[4px] bg-blue-500/15 text-blue-700 dark:bg-blue-400/15 dark:text-blue-300",
  cmd: "rounded-[4px] bg-violet-500/15 text-violet-700 dark:bg-violet-400/15 dark:text-violet-300",
  skill: "rounded-[4px] bg-amber-500/15 text-amber-700 dark:bg-amber-400/15 dark:text-amber-300",
} as const;

/** Render composer text with `@file`/`/cmd`/`$skill` tokens as chips.
 * Mirrors the kernel's `expand_user_tokens` rules: `@` and `$` tokens
 * anywhere (whitespace/start bounded — mid-text `$name` inlines the
 * skill body), `/` only at position 0 where it's a real command. */
function highlightComposerTokens(text: string) {
  const nodes: (string | JSX.Element)[] = [];
  // `\S+` tokens — the kernel's `@`/`/`/`$` tokens run to the next
  // whitespace, so paths containing `/`, `$`, or even `@` mid-token
  // stay one chip. `(?!x)` rejects doubled triggers (`@@`, `//`, `$$`),
  // matching the kernel which re-scans past the literal first char.
  const re = /@(?!@)\S+|\$(?!\$)\S+|\/(?!\/)\S+/g;
  let m: RegExpExecArray | null;
  let last = 0;
  let key = 0;
  while ((m = re.exec(text))) {
    const token = m[0];
    const ch = token[0];
    const start = m.index;
    if (ch === "@" || ch === "$") {
      // `a@b.com` / `x$HOME` stay literal — need a whitespace/start
      // boundary, or a doubled trigger the kernel re-scans (`@@`, `$$`).
      if (start > 0 && !/\s/.test(text[start - 1]) && text[start - 1] !== ch)
        continue;
      // `$5`/`$(x)` can't be skill names — the kernel skips the lookup.
      if (ch === "$" && !/^[A-Za-z_]/.test(token.slice(1))) continue;
    } else if (start !== 0) {
      continue;
    }
    if (token.length <= 1) continue; // bare trigger char
    nodes.push(text.slice(last, start));
    const kind = ch === "@" ? "file" : ch === "$" ? "skill" : "cmd";
    nodes.push(
      <span key={key++} className={TOKEN_CHIP_CLS[kind]}>
        {token}
      </span>,
    );
    last = start + token.length;
  }
  nodes.push(text.slice(last));
  return nodes;
}

/** Mirror layer — paints the token chips under the transparent-text
 * textarea. Its box metrics (padding/font/line-height) must track the
 * Textarea's exactly or the chips drift off the glyphs. */
export function TokenMirror({
  value,
  mirrorRef,
}: {
  value: string;
  mirrorRef: RefObject<HTMLDivElement>;
}) {
  return (
    <div
      ref={mirrorRef}
      aria-hidden
      className="pointer-events-none select-none absolute inset-0 overflow-hidden whitespace-pre-wrap break-words px-2 pt-1 pb-2 text-[14px] leading-relaxed text-neutral-900"
    >
      {highlightComposerTokens(value)}
      {"\u200B"}
    </div>
  );
}
