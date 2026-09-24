import { useDeferredValue, useEffect, useRef, useState } from "react";
import { parseMarkdownIntoBlocks } from "streamdown";
import { MemoStreamdown } from "./markdown-stream";
import { thinkingMarkdownComponents } from "./markdown-components";

/** Reasoning is the only unbounded markdown surface — a long trace (200k+
 *  chars) expanded in one Streamdown pass pays a full remend + lex + DOM
 *  commit in a single frame, plus Radix's measured-height animation on top.
 *  This component paginates the text at markdown block boundaries and mounts
 *  pages progressively, so an expand mounts ~2 pages and a stream delta
 *  re-lexes only the unfrozen tail instead of the whole trace. */

const PAGE_CHARS = 12_000;
const HUGE_BLOCK = PAGE_CHARS * 2;
const INITIAL_PAGES = 2;
const SENTINEL_MARGIN = "800px 0px";

/** A single oversized block (a giant fence) is split at line boundaries for
 *  display only — chunks stay verbatim substrings so the consumed-offset
 *  bookkeeping still locates them in the raw text. Fence context across
 *  chunks is the accepted trade-off. */
function splitHugeBlock(block: string): string[] {
  if (block.length <= HUGE_BLOCK) return [block];
  const lines = block.split("\n");
  const chunks: string[] = [];
  let cur = "";
  for (const line of lines) {
    if (cur && cur.length + line.length + 1 > PAGE_CHARS) {
      chunks.push(cur);
      cur = "";
    }
    cur = cur ? `${cur}\n${line}` : line;
  }
  if (cur) chunks.push(cur);
  return chunks;
}

type PagesCache = { consumed: number; frozen: string[] };

/** Fold `text` into ~PAGE_CHARS pages. `consumed` records the raw offset the
 *  frozen pages came from, so each delta re-lexes only the tail. A page
 *  freezes only when it ends at a complete-block boundary — the last block
 *  may still be growing, and freezing mid-block would make the remainder
 *  vanish (the tail re-parse restarts at `consumed`). */
function useReasoningPages(text: string): string[] {
  const ref = useRef<PagesCache>({ consumed: 0, frozen: [] });
  if (text.length < ref.current.consumed) {
    // The row's text reset (phase switch on the same component) — rebuild.
    ref.current = { consumed: 0, frozen: [] };
  }
  const cache = ref.current;
  const tail = text.slice(cache.consumed);
  const blocks = parseMarkdownIntoBlocks(tail);

  // Raw start offset of each block in the tail — blocks are verbatim
  // substrings, so indexOf from a forward-only cursor locates them even
  // when identical text repeats.
  const blockStarts: number[] = [];
  let cursor = 0;
  for (const b of blocks) {
    const at = tail.indexOf(b, cursor);
    const start = at === -1 ? cursor : at;
    blockStarts.push(start);
    cursor = start + b.length;
  }

  const units: { t: string; b: number }[] = [];
  blocks.forEach((b, bi) =>
    splitHugeBlock(b).forEach((t) => units.push({ t, b: bi })),
  );

  const livePages: string[] = [];
  const pageEndUnit: number[] = [];
  let page = "";
  units.forEach((u, ui) => {
    if (page && page.length + u.t.length > PAGE_CHARS) {
      livePages.push(page);
      pageEndUnit.push(ui - 1);
      page = "";
    }
    page = page ? `${page}\n\n${u.t}` : u.t;
  });
  if (page) {
    livePages.push(page);
    pageEndUnit.push(units.length - 1);
  }

  // Freezable prefix: a page is final iff its last unit is the last unit of
  // a complete (non-head) block.
  let seal = 0;
  for (let i = 0; i < livePages.length; i++) {
    const endU = pageEndUnit[i];
    const atBlockEnd =
      endU + 1 >= units.length || units[endU + 1].b !== units[endU].b;
    const blockComplete = units[endU].b < blocks.length - 1;
    if (!atBlockEnd || !blockComplete) break;
    seal = i + 1;
  }
  if (seal > 0) {
    const lastB = units[pageEndUnit[seal - 1]].b;
    cache.consumed += blockStarts[lastB] + blocks[lastB].length;
    cache.frozen.push(...livePages.slice(0, seal));
    livePages.splice(0, seal);
  }
  return cache.frozen.concat(livePages);
}

/** Paginated reasoning body — drop-in for the old single-Streamdown body.
 *  Frozen pages get `animating=false` so their code blocks skip the
 *  streaming highlighter; only the live head page animates. */
export const ReasoningBody = function ReasoningBody({
  text,
  animating,
}: {
  text: string;
  animating?: boolean;
}) {
  // Deferred text: many deltas/second collapse into low-priority renders
  // that React can interrupt — typing never waits on a re-lex.
  const deferred = useDeferredValue(text);
  const pages = useReasoningPages(deferred);
  const [mounted, setMounted] = useState(INITIAL_PAGES);
  const sentinelRef = useRef<HTMLDivElement | null>(null);
  const shown = Math.min(mounted, pages.length);

  useEffect(() => {
    const el = sentinelRef.current;
    if (!el || shown >= pages.length) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          setMounted((m) => Math.min(m + 1, pages.length));
        }
      },
      { rootMargin: SENTINEL_MARGIN },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [shown, pages.length]);

  return (
    <div className="text-neutral-500 mt-1 w-full min-w-0 text-[13px] leading-relaxed select-text font-normal pl-6">
      {pages.slice(0, shown).map((page, i) => (
        <MemoStreamdown
          key={i}
          text={page}
          animating={animating && i === pages.length - 1}
          components={thinkingMarkdownComponents}
        />
      ))}
      {shown < pages.length && (
        <div ref={sentinelRef} className="pt-1">
          <button
            type="button"
            onClick={() => setMounted((m) => Math.min(m + 4, pages.length))}
            className="text-xs text-neutral-400 hover:text-neutral-600"
          >
            加载更多思考…({pages.length - shown} 段)
          </button>
        </div>
      )}
    </div>
  );
};
