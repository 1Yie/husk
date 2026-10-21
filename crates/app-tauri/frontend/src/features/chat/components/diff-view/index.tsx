// DiffView — the file-diff card: one card per touched file, a header naming the
// path beside its ±line totals, a two-column line-number gutter, and rows tinted
// by kind (additions carry a solid green edge, deletions a red hatch). Changed
// code is coloured through the app's own tokenizer.
//
// The input stays a unified diff string: a write tool's result, an approval
// preview or a ```diff fence all arrive in that shape.

import { useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import { highlightCodeToHtml, normalizeLanguage, supportsLanguage } from "@/lib/syntax-highlight";

interface Props {
  diff: string;
  maxHeight?: number;
  /** Lay out the horizontal scrollbar track unconditionally.
   *
   *  WebKit only recomputes an `auto` scrollbar on a layout pass, so a diff
   *  that mounts while its containing panel animates open can sit there with
   *  no horizontal bar until something else forces a re-layout. A `scroll`
   *  track is laid out from the first frame; the thumb still only shows when
   *  the content overflows. Opt-in because it costs a permanent 6px gutter —
   *  worth it in a resizable dock, needless in a chat bubble. */
  reserveScrollbar?: boolean;
  /** The card's own header — file path and ±totals. Turn it off where the
   *  surrounding row already names the file, as the dock's file row does. */
  showHeader?: boolean;
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

export function DiffView({
  diff,
  maxHeight = 500,
  reserveScrollbar = false,
  showHeader = true,
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
        out.push({ ...section, rows: section.rows.slice(0, budget) });
        budget = 0;
      }
    }
    return out;
  }, [capped, sections]);

  if (sections.length === 0 || totalRows === 0) return null;

  return (
    <div className="w-full min-w-0">
      <div
        className="flex w-full min-w-0 flex-col gap-2 overflow-x-hidden overflow-y-auto"
        style={{ maxHeight }}
      >
        {shown.map((section, i) => (
          <FileDiffCard
            key={`${section.path ?? "diff"}-${i}`}
            section={section}
            showHeader={showHeader}
            reserveScrollbar={reserveScrollbar}
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
}

const MAX_DIFF_LINES = 200;

function FileDiffCard({
  section,
  showHeader,
  reserveScrollbar,
}: {
  section: Section;
  showHeader: boolean;
  reserveScrollbar: boolean;
}) {
  const html = useMemo(() => {
    if (!section.language || section.code.length === 0) return undefined;
    // One tokenize pass for the whole file's changed lines; the renderer's
    // output is per-line balanced, so splitting it back per row is exact.
    return highlightCodeToHtml(section.code.join("\n"), section.language).split("\n");
  }, [section]);

  return (
    <div
      className={cn(
        "w-full min-w-0 select-text overflow-hidden rounded-[12px] bg-white font-mono",
        "shadow-[0_1px_2px_rgba(0,0,0,0.05),0_2px_4px_rgba(0,0,0,0.02),0_0_0_0.5px_rgba(0,0,0,0.08)]",
        "dark:shadow-[0_1px_2px_rgba(0,0,0,0.4),0_2px_4px_rgba(0,0,0,0.3),0_0_0_0.5px_rgba(255,255,255,0.12)]",
      )}
    >
      {showHeader && (
        <div className="flex items-center gap-2 border-b-[0.5px] border-neutral-200 px-3 py-2.5 text-[12.5px] leading-none">
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
          {section.path ? (
            <span className="min-w-0 truncate text-neutral-900" title={section.path}>
              {section.path}
            </span>
          ) : null}
          <span className="ml-auto flex flex-none items-center gap-2 text-[12px] leading-none">
            <span className="text-[#15a06a] dark:text-[#34d399]">+{section.adds}</span>
            <span className="text-[#dc2626] dark:text-[#f87171]">-{section.dels}</span>
          </span>
        </div>
      )}

      <div className={cn("w-full", reserveScrollbar ? "overflow-x-scroll" : "overflow-x-auto")}>
        {/* The gutter divider lives on the inner track so it stays aligned to
            the line numbers while the body scrolls. */}
        <div
          className={cn(
            "relative w-max min-w-full py-1 text-[12.5px] leading-5",
            "before:absolute before:inset-y-0 before:left-16 before:z-[1] before:w-[0.5px]",
            "before:bg-[#e6e8eb] before:content-[''] dark:before:bg-[#303030]",
          )}
        >
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
}

function DiffRow({ row, html }: { row: Row; html?: string }) {
  if (row.kind === "hunk") {
    return (
      <div className="select-none whitespace-pre bg-[color-mix(in_srgb,var(--husk-n100)_70%,transparent)] px-3 py-px text-[11px] text-neutral-500">
        {row.text}
      </div>
    );
  }

  if (row.kind === "meta") {
    return (
      <div className="select-none whitespace-pre px-3 text-[11px] italic text-neutral-500">
        {row.text}
      </div>
    );
  }

  const codeClass = cn(
    "min-w-0 whitespace-pre pl-2 pr-3",
    row.kind === "ctx" ? "text-[#a1a1a1]" : "text-neutral-900",
  );

  return (
    <div
      className={cn(
        "relative grid grid-cols-[32px_32px_18px_1fr] items-stretch",
        ROW_TONE[row.kind],
      )}
    >
      <span
        className={cn(
          "select-none px-[7px] text-right text-[11px]",
          row.kind === "del" ? "text-[#dc2626] dark:text-[#f87171]" : "text-[#a1a1a1]",
        )}
      >
        {row.old ?? ""}
      </span>
      <span
        className={cn(
          "select-none px-[7px] text-right text-[11px]",
          row.kind === "add" ? "text-[#15a06a] dark:text-[#34d399]" : "text-[#a1a1a1]",
        )}
      >
        {row.cur ?? ""}
      </span>
      <span
        className={cn(
          "select-none text-center text-[11px]",
          row.kind === "add"
            ? "text-[#15a06a] dark:text-[#34d399]"
            : row.kind === "del"
              ? "text-[#dc2626] dark:text-[#f87171]"
              : "text-[#a1a1a1]",
        )}
      >
        {row.kind === "add" ? "+" : row.kind === "del" ? "-" : ""}
      </span>
      {html === undefined ? (
        <span className={codeClass}>{row.text || "\u00A0"}</span>
      ) : (
        <span className={codeClass} dangerouslySetInnerHTML={{ __html: html || "\u00A0" }} />
      )}
    </div>
  );
}

/** Row tints and edge accents: solid green for an addition, red hatch for a
 *  deletion, nothing for unchanged lines. */
const ROW_TONE: Record<"add" | "del" | "ctx", string> = {
  add: [
    "bg-[rgba(26,127,55,0.09)] dark:bg-[rgba(63,185,80,0.15)]",
    "before:absolute before:inset-y-0 before:left-0 before:w-[3px] before:content-['']",
    "before:bg-[#15a06a] dark:before:bg-[#34d399]",
  ].join(" "),
  del: [
    "bg-[rgba(207,34,46,0.09)] dark:bg-[rgba(248,81,73,0.15)]",
    "before:absolute before:inset-y-0 before:left-0 before:w-[3px] before:content-['']",
    "before:bg-[repeating-linear-gradient(45deg,#dc2626_0,#dc2626_1.5px,transparent_1.5px,transparent_3px)]",
    "dark:before:bg-[repeating-linear-gradient(45deg,#f87171_0,#f87171_1.5px,transparent_1.5px,transparent_3px)]",
  ].join(" "),
  ctx: "",
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
      inHunk = true;
      oldNo = hunk ? Number(hunk[1]) : 0;
      newNo = hunk ? Number(hunk[2]) : 0;
      current().rows.push({ kind: "hunk", text: line });
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
    return m ? (cleanPath(m[2]) ?? null) : null;
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
