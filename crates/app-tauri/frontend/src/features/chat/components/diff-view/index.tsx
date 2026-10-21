// DiffView — the file-diff card: one card per touched file, a header naming the
// path beside its ±line totals, a two-column line-number gutter, and rows tinted
// by kind (additions carry a solid green edge, deletions a red hatch). Changed
// code is coloured through the app's own tokenizer.
//
// The input stays a unified diff string: a write tool's result, an approval
// preview or a ```diff fence all arrive in that shape.

import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import {
  highlightCodeToLines,
  normalizeLanguage,
  peekHighlightedLines,
  scheduleHighlightLines,
  supportsLanguage,
} from "@/lib/syntax-highlight";

interface Props {
  diff: string;
  maxHeight?: number;
  /** The file this diff belongs to, when the diff text itself does not say
   *  (the dock slices a patch down to its hunks). Names the card and picks
   *  the highlighter; a header inside the diff still wins. */
  file?: string;
}

type RowKind = "add" | "del" | "ctx" | "hunk" | "meta";

interface Row {
  kind: RowKind;
  text: string;
  old?: number;
  cur?: number;
  /** Index into `Section.code` — the line the highlighter colours. */
  code?: number;
}

interface Section {
  path?: string;
  rows: Row[];
  code: string[];
  adds: number;
  dels: number;
  language?: string;
}

export const DiffView = memo(function DiffView({
  diff,
  maxHeight = 500,
  file,
}: Props) {
  const [expanded, setExpanded] = useState(false);
  const sections = useMemo(() => parseDiff(diff), [diff]);
  const totalRows = useMemo(
    () => sections.reduce((n, section) => n + section.rows.length, 0),
    [sections],
  );
  // A giant diff must not mount tens of thousands of nodes: the scroll box
  // clips the visual height, the cap clips the DOM. Explicit click-to-expand.
  const capped = !expanded && totalRows > MAX_DIFF_LINES;
  const shown = useMemo(() => {
    if (!capped) return sections;
    let budget = MAX_DIFF_LINES;
    const out: Section[] = [];
    for (const section of sections) {
      if (budget <= 0) break;
      if (section.rows.length <= budget) {
        out.push(section);
        budget -= section.rows.length;
      } else {
        const rows = section.rows.slice(0, budget);
        // `row.code` indexes `code`: a 5k-line file must not be tokenized in
        // full to paint its first 200 rows, so cut the highlighter's input at
        // the last visible line.
        let lastCode = -1;
        for (const row of rows) if (row.code !== undefined) lastCode = row.code;
        out.push({ ...section, rows, code: section.code.slice(0, lastCode + 1) });
        budget = 0;
      }
    }
    return out;
  }, [capped, sections]);

  if (sections.length === 0 || totalRows === 0) return null;

  return (
    <div className="w-full min-w-0">
      {/* No shared scroll box: every card caps and scrolls its own body, so a
          card's horizontal bar sits at the bottom of what is on screen. Under
          a shared vertical box, a tall card's bar was at its far end — out of
          reach until the box was scrolled down past the whole file. */}
      <div className="flex w-full min-w-0 flex-col gap-2">
        {shown.map((section, i) => (
          <FileDiffCard
            key={`${section.path ?? "diff"}-${i}`}
            section={section}
            file={file}
            maxHeight={maxHeight}
          />
        ))}
      </div>
      {capped && (
        <button
          type="button"
          onClick={() => setExpanded(true)}
          className="w-full select-none py-1.5 text-center font-mono text-[11px] text-neutral-500 hover:text-neutral-600"
        >
          … 还有 {totalRows - MAX_DIFF_LINES} 行，点击展开全部
        </button>
      )}
    </div>
  );
});

const MAX_DIFF_LINES = 200;

const FileDiffCard = memo(function FileDiffCard({
  section,
  file,
  maxHeight,
}: {
  section: Section;
  file?: string;
  /** Cap for the whole card. The card is a flex column, so the body shrinks
   *  into whatever the header leaves — no hand-kept chrome height to drift. */
  maxHeight: number;
}) {
  const title = section.path ?? file;
  const language = section.language ?? languageOf(file);
  const html = useHighlightedLines(section.code, language);
  const { ref: scrollRef, overflowing } = useOverflowX<HTMLDivElement>();

  return (
    <div
      className={cn(
        "flex w-full min-w-0 select-text flex-col overflow-hidden rounded-[12px] border border-hairline bg-white font-mono",
        // A dock resize or a sibling's expand must not re-lay out every row
        // of every other card.
        "[contain:content]",
        // The page is white in the light theme, so the reference's sub-pixel
        // ring washes out — a real hairline plus a soft drop keeps the card
        // readable as a surface.
        "shadow-[0_1px_2px_rgba(0,0,0,0.05),0_2px_4px_rgba(0,0,0,0.02)]",
        "dark:shadow-[0_1px_2px_rgba(0,0,0,0.4),0_2px_4px_rgba(0,0,0,0.3)]",
      )}
      style={{ maxHeight }}
    >
      <div className="flex flex-none items-center gap-2 border-b border-neutral-200 px-3 py-2.5 text-[12.5px] leading-none">
        <svg
          aria-hidden="true"
          className="h-[15px] w-[15px] flex-none text-[#a1a1a1]"
          fill="none"
          viewBox="0 0 24 24"
        >
          <path
            d="M17.25 6.75 22.5 12l-5.25 5.25m-10.5 0L1.5 12l5.25-5.25m7.5-3-4.5 16.5"
            stroke="currentColor"
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.8}
          />
        </svg>
        {title ? (
          <span className="min-w-0 truncate text-neutral-900" title={title}>
            {title}
          </span>
        ) : null}
        <span className="ml-auto flex flex-none items-center gap-2 text-[12px] leading-none">
          <span className="text-[#15a06a] dark:text-[#34d399]">+{section.adds}</span>
          <span className="text-[#dc2626] dark:text-[#f87171]">-{section.dels}</span>
        </span>
      </div>

      <div
        ref={scrollRef}
        className={cn(
          "min-h-0 w-full flex-1 overflow-y-auto",
          // The bar's 6px is held as padding until a line overflows, so the
          // card is the same height either way. Highlighting lands after
          // mount and widens lines (bold keywords, italic comments); without
          // this, the bar appearing grew the card under the reader — and
          // WebKit has no scroll anchoring to absorb it.
          overflowing ? "overflow-x-auto" : "overflow-x-hidden pb-1.5",
        )}
      >
        {/* The gutter is sticky per row, so line numbers stay put while the
            code scrolls sideways — in a 260px dock, the first swipe used to
            take every number with it. */}
        <div className="w-max min-w-full py-1 text-[12.5px] leading-5">
          {section.rows.map((row, i) => (
            <DiffRow
              key={i}
              row={row}
              html={row.code === undefined ? undefined : html?.[row.code]}
            />
          ))}
        </div>
      </div>
    </div>
  );
});

/** Colour a card of at most this many code lines before paint. A bigger diff
 *  mounts as plain text (same rows, same height) and gets its colours from the
 *  shared highlight queue. */
const SYNC_HIGHLIGHT_LINES = 80;
/** …and at most this many lines across every card in one commit: the dock
 *  opens on a turn's first change with all its small patches expanded, and a
 *  per-card cap alone still tokenized them all back to back. */
const SYNC_HIGHLIGHT_BUDGET = 160;
let syncSpent = 0;
let syncResetPending = false;

function takeSyncBudget(lines: number): boolean {
  if (syncSpent + lines > SYNC_HIGHLIGHT_BUDGET) return false;
  syncSpent += lines;
  if (!syncResetPending) {
    // A commit renders in one task; the budget refills for the next one.
    syncResetPending = true;
    setTimeout(() => {
      syncSpent = 0;
      syncResetPending = false;
    }, 0);
  }
  return true;
}

function useHighlightedLines(code: string[], language?: string): string[] | undefined {
  const source = useMemo(() => code.join("\n"), [code]);
  const sync = useMemo(() => {
    if (!language || !source) return undefined;
    // A card folded and reopened, or the same patch in the stream and the
    // dock, is already coloured — that costs nothing to paint.
    const cached = peekHighlightedLines(source, language);
    if (cached) return cached;
    if (code.length <= SYNC_HIGHLIGHT_LINES && takeSyncBudget(code.length)) {
      return highlightCodeToLines(source, language);
    }
    return undefined;
  }, [code, language, source]);
  const [deferred, setDeferred] = useState<{ source: string; lines: string[] } | null>(null);

  useEffect(() => {
    if (sync || !language || !source) return;
    return scheduleHighlightLines(source, language, (lines) => setDeferred({ source, lines }));
  }, [sync, language, source]);

  if (sync) return sync;
  return deferred && deferred.source === source ? deferred.lines : undefined;
}

/** `overflow-x` is on only while a line is wider than the body.
 *
 *  A `scroll` track was reserved to dodge WebKit laying out an `auto`
 *  scrollbar only on a layout pass — a diff mounting during the dock's width
 *  transition kept a stale, missing bar. That cost a permanent 6px track whose
 *  thumb painted even with nothing to scroll. Measuring instead produces the
 *  hidden→auto style change (a layout pass) exactly when the content needs
 *  one. */
function useOverflowX<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [overflowing, setOverflowing] = useState(false);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => {
      // Both sides are rounded integers: a fractional dock width can read one
      // pixel over with nothing to scroll, which painted a 1px-travel bar.
      const next = el.scrollWidth - el.clientWidth > 1;
      setOverflowing((prev) => (prev === next ? prev : next));
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    // The track is `w-max`: a long line added by the highlight or by expanding
    // a capped diff changes its width, not the scrollport's.
    if (el.firstElementChild) observer.observe(el.firstElementChild);
    return () => observer.disconnect();
  }, []);

  return { ref, overflowing };
}

const DiffRow = memo(function DiffRow({ row, html }: { row: Row; html?: string }) {
  // Full-width bands: the tint spans the whole track, the label inside sticks
  // to the scrollport's left edge like the line-number gutter does. The band
  // itself cannot stick — it is as wide as the track, so it has no room to.
  if (row.kind === "hunk") {
    return (
      <div className="select-none whitespace-pre bg-[color-mix(in_srgb,var(--husk-n100)_70%,transparent)] px-3 py-px text-[11px] text-neutral-500">
        <span className="sticky left-3 inline-block">{row.text}</span>
      </div>
    );
  }

  if (row.kind === "meta") {
    return (
      <div className="select-none whitespace-pre px-3 text-[11px] italic text-neutral-500">
        <span className="sticky left-3 inline-block">{row.text}</span>
      </div>
    );
  }

  const codeClass = cn(
    "min-w-0 whitespace-pre pl-2 pr-3",
    row.kind === "ctx" ? "text-[#a1a1a1]" : "text-neutral-900",
  );

  return (
    <div className={cn("grid grid-cols-[82px_1fr] items-stretch bg-[var(--row-tint)]", ROW_TONE[row.kind])}>
      {/* Opaque, so code scrolled under it does not show through: the row's
          tint laid over the card colour reads the same as the tint alone. */}
      <div
        className={cn(
          "sticky left-0 z-[1] grid select-none grid-cols-[32px_32px_18px] text-[11px]",
          "[background:linear-gradient(var(--row-tint),var(--row-tint)),var(--husk-white)]",
          "after:absolute after:inset-y-0 after:left-16 after:w-px after:content-['']",
          "after:bg-[#e6e8eb] dark:after:bg-[#303030]",
        )}
      >
        <span
          className={cn(
            "px-[7px] text-right",
            row.kind === "del" ? "text-[#dc2626] dark:text-[#f87171]" : "text-[#a1a1a1]",
          )}
        >
          {row.old ?? ""}
        </span>
        <span
          className={cn(
            "px-[7px] text-right",
            row.kind === "add" ? "text-[#15a06a] dark:text-[#34d399]" : "text-[#a1a1a1]",
          )}
        >
          {row.cur ?? ""}
        </span>
        <span
          className={cn(
            "text-center",
            row.kind === "add"
              ? "text-[#15a06a] dark:text-[#34d399]"
              : row.kind === "del"
                ? "text-[#dc2626] dark:text-[#f87171]"
                : "text-[#a1a1a1]",
          )}
        >
          {row.kind === "add" ? "+" : row.kind === "del" ? "-" : ""}
        </span>
      </div>
      {html === undefined ? (
        <span className={codeClass}>{row.text || "\u00A0"}</span>
      ) : (
        <span className={codeClass} dangerouslySetInnerHTML={{ __html: html || "\u00A0" }} />
      )}
    </div>
  );
});

/** Row tints (`--row-tint`, shared by the row and its sticky gutter) and the
 *  gutter's edge accent: solid green for an addition, red hatch for a
 *  deletion, nothing for unchanged lines. */
const ROW_TONE: Record<"add" | "del" | "ctx", string> = {
  add: [
    "[--row-tint:rgba(26,127,55,0.09)] dark:[--row-tint:rgba(63,185,80,0.15)]",
    "[&>div]:before:absolute [&>div]:before:inset-y-0 [&>div]:before:left-0 [&>div]:before:w-[3px] [&>div]:before:content-['']",
    "[&>div]:before:bg-[#15a06a] dark:[&>div]:before:bg-[#34d399]",
  ].join(" "),
  del: [
    "[--row-tint:rgba(207,34,46,0.09)] dark:[--row-tint:rgba(248,81,73,0.15)]",
    "[&>div]:before:absolute [&>div]:before:inset-y-0 [&>div]:before:left-0 [&>div]:before:w-[3px] [&>div]:before:content-['']",
    "[&>div]:before:bg-[repeating-linear-gradient(45deg,#dc2626_0,#dc2626_1.5px,transparent_1.5px,transparent_3px)]",
    "dark:[&>div]:before:bg-[repeating-linear-gradient(45deg,#f87171_0,#f87171_1.5px,transparent_1.5px,transparent_3px)]",
  ].join(" "),
  ctx: "[--row-tint:transparent]",
};

/** Metadata git emits between hunks — never rendered. */
const FILE_META =
  /^(index |new file mode |deleted file mode |old mode |new mode |similarity index |dissimilarity index |rename from |rename to |GIT binary patch|Binary files )/;

function parseDiff(diff: string): Section[] {
  const sections: Section[] = [];
  let section: Section | null = null;
  let oldNo = 0;
  let newNo = 0;
  let inHunk = false;

  const open = (path?: string): Section => {
    section = { path, rows: [], code: [], adds: 0, dels: 0, language: languageOf(path) };
    sections.push(section);
    inHunk = false;
    return section;
  };
  // A plain read: TS narrows `section` to its initial `null` otherwise, since
  // the assignments happen inside the closure below.
  const peek = (): Section | null => section;
  const current = (): Section => section ?? open();

  for (const raw of diff.split("\n")) {
    const line = raw.replace(/\r$/, "");
    const trimmed = line.trim();
    if (!trimmed || trimmed === "*** Begin Patch" || trimmed === "*** End Patch") continue;

    const header = fileHeader(line);
    if (header !== undefined) {
      open(header ?? undefined);
      continue;
    }

    if (line.startsWith("--- ") || line.startsWith("+++ ")) {
      // apply_patch names a hunk's ends `a/file` / `b/file` when the diff text
      // carries no real path. The pair is a placeholder, not a filename.
      if (/^(?:---|\+\+\+) [ab]\/file$/.test(line)) continue;
      // `--- a/x` / `+++ b/x` name the file when no `diff --git` row did. A
      // deleted line that happens to start with `-- ` is only read as content
      // once a hunk is open and the line is not the a/ b/ form.
      if (!inHunk || /^(---|\+\+\+) [ab]\//.test(line)) {
        const path = cleanPath(line.slice(4));
        const active = peek();
        if (!active) {
          open(path);
        } else if (!active.path && path) {
          active.path = path;
          active.language = languageOf(path);
        }
        continue;
      }
    }

    if (line.startsWith("@@")) {
      const hunk = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
      // Section first: a headerless diff (the dock's hunk-only slices) opens
      // its section here, and `open()` resets `inHunk`.
      const target = current();
      inHunk = true;
      oldNo = hunk ? Number(hunk[1]) : 0;
      newNo = hunk ? Number(hunk[2]) : 0;
      target.rows.push({ kind: "hunk", text: line });
      continue;
    }

    if (line.startsWith("\\")) continue; // "\ No newline at end of file"
    if (FILE_META.test(line)) continue;

    const target = current();
    if (line.startsWith("+")) {
      const text = line.slice(1);
      target.rows.push({
        kind: "add",
        text,
        cur: inHunk ? newNo++ : undefined,
        code: target.code.length,
      });
      target.code.push(text);
      target.adds += 1;
    } else if (line.startsWith("-")) {
      const text = line.slice(1);
      target.rows.push({
        kind: "del",
        text,
        old: inHunk ? oldNo++ : undefined,
        code: target.code.length,
      });
      target.code.push(text);
      target.dels += 1;
    } else if (line.startsWith(" ")) {
      const text = line.slice(1);
      target.rows.push({
        kind: "ctx",
        text,
        old: inHunk ? oldNo++ : undefined,
        cur: inHunk ? newNo++ : undefined,
        code: target.code.length,
      });
      target.code.push(text);
    } else {
      target.rows.push({ kind: "meta", text: line });
    }
  }

  return sections;
}

/** File header row → path (`null` = header with no usable path, `undefined` =
 *  not a header). */
function fileHeader(line: string): string | null | undefined {
  if (line.startsWith("*** ")) {
    const m = /^\*\*\*\s*(?:Add|Update|Delete)\s+File:\s*(.+)$/.exec(line);
    return m ? (cleanPath(m[1]) ?? null) : undefined;
  }
  if (line.startsWith("diff --git ")) {
    const m = /^diff --git a\/(.+?) b\/(.+)$/.exec(line);
    if (!m) return null;
    // The tool's placeholder pair, not a real filename.
    if (m[1] === "file" && m[2] === "file") return null;
    return cleanPath(m[2]) ?? null;
  }
  if (line.startsWith("Index: ")) return cleanPath(line.slice(7)) ?? null;
  const m = /^(?:added|deleted|modified|updated|patched)\s+(.+)$/.exec(line);
  if (m) return cleanPath(m[1]) ?? null;
  return undefined;
}

function cleanPath(raw: string): string | undefined {
  let path = raw.trim();
  if (path.startsWith('"') && path.endsWith('"')) path = path.slice(1, -1);
  // `fuzzy_patch` prefixes its diff with `patched {path} (line N)`.
  path = path.replace(/\s+\(line \d+\)$/, "").trim();
  if (!path || path === "/dev/null") return undefined;
  path = path.replace(/^[ab]\//, "");
  return path || undefined;
}

function languageOf(path?: string): string | undefined {
  if (!path) return undefined;
  const base = path.slice(path.lastIndexOf("/") + 1);
  const dot = base.lastIndexOf(".");
  if (dot <= 0) return undefined;
  const language = normalizeLanguage(base.slice(dot + 1));
  return supportsLanguage(language) ? language : undefined;
}
