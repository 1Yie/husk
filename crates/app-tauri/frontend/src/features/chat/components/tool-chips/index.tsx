import { memo, useMemo, useState } from "react";
import { ChevronDown } from "@keyline-icons/react";
import { AgentCard } from "@/features/chat/components/tool-chips/agent-card";
import { DiffView } from "@/features/chat/components/diff-view/index";
import { RowDetail } from "@/features/chat/components/tool-chips/row-detail";
import type { ChildSegment } from "@/features/chat/hooks/stream-view";
import { TodoView } from "@/features/chat/components/todo-view/index";
import { highlightCodeToHtml } from "@/lib/syntax-highlight";
import { cn } from "@/lib/utils";
import { TooltipSimple } from "@/components/ui/tooltip";

function detectLanguageFromPath(path: string): string {
  const clean = path.toLowerCase().trim();
  if (clean.endsWith(".ts")) return "typescript";
  if (clean.endsWith(".tsx")) return "tsx";
  if (clean.endsWith(".js") || clean.endsWith(".mjs") || clean.endsWith(".cjs")) return "javascript";
  if (clean.endsWith(".jsx")) return "jsx";
  if (clean.endsWith(".rs")) return "rust";
  if (clean.endsWith(".py") || clean.endsWith(".pyw") || clean.endsWith(".pyi")) return "python";
  if (clean.endsWith(".go")) return "go";
  if (clean.endsWith(".json")) return "json";
  if (clean.endsWith(".toml")) return "toml";
  if (clean.endsWith(".yaml") || clean.endsWith(".yml")) return "yaml";
  if (clean.endsWith(".md") || clean.endsWith(".markdown")) return "markdown";
  if (clean.endsWith(".css")) return "css";
  if (clean.endsWith(".scss") || clean.endsWith(".sass")) return "scss";
  if (clean.endsWith(".html") || clean.endsWith(".htm")) return "markup";
  if (clean.endsWith(".sh") || clean.endsWith(".bash") || clean.endsWith(".zsh")) return "bash";
  if (clean.endsWith(".sql")) return "sql";
  if (clean.endsWith(".c") || clean.endsWith(".h")) return "c";
  if (clean.endsWith(".cpp") || clean.endsWith(".hpp") || clean.endsWith(".cc") || clean.endsWith(".cxx")) return "cpp";
  if (clean.endsWith(".cs")) return "csharp";
  if (clean.endsWith(".java")) return "java";
  if (clean.endsWith(".kt") || clean.endsWith(".kts")) return "kotlin";
  if (clean.endsWith(".swift")) return "swift";
  if (clean.endsWith(".xml") || clean.endsWith(".svg")) return "markup";
  if (clean.endsWith(".dockerfile") || clean.includes("dockerfile")) return "docker";
  if (clean.endsWith(".env") || clean.includes(".env")) return "bash";
  return "";
}

function extractFilePath(chip: string): string {
  if (!chip) return "";
  try {
    const parsed = JSON.parse(chip);
    if (typeof parsed === "string") return parsed;
    if (parsed.path) return String(parsed.path);
    if (parsed.file) return String(parsed.file);
    if (parsed.filename) return String(parsed.filename);
  } catch {
  }
  const match = chip.match(/([a-zA-Z0-9_\-./\\]+\.[a-zA-Z0-9]+)/);
  if (match) return match[1];
  return chip;
}

function getToolLanguage(detailText: string, label: string, chip: string): string {
  const firstLine = detailText.split("\n")[0] || "";
  const headerMatch = firstLine.match(/^([^\s\n\r(]+\.[a-zA-Z0-9]+)/);
  if (headerMatch) {
    const l = detectLanguageFromPath(headerMatch[1]);
    if (l) return l;
  }
  const filePath = extractFilePath(chip);
  if (filePath) {
    const l = detectLanguageFromPath(filePath);
    if (l) return l;
  }
  if (label === "bash" || label === "pty" || label === "shell") {
    return "bash";
  }
  return "text";
}

/** Cap the mounted line count inside a capsule — the scroll box clips
 *  the visual height but not the DOM, and a multi-thousand-line tool
 *  output is tens of thousands of nodes (that single-handedly froze the
 *  whole webview). User expands explicitly — no scroll-time mounting. */
const MAX_DETAIL_LINES = 150;
/// Expanding reveals in steps, not all-at-once: a single "展开全部" click
/// on a 10k-line output mounted ~10k nodes + ~10k highlight passes in one
/// frame (the expand-time freeze). Stepped reveal bounds each click's mount
/// cost; per-line memoization keeps already-rendered lines from re-highlighting.
const EXPAND_STEP = 500;

/** One rendered output line — memoized per (line, lang) so a reveal click
 *  only pays highlight cost for the newly mounted page, not the whole body. */
const HighlightedLine = memo(function HighlightedLine({
  line,
  lang,
}: {
  line: string;
  lang: string;
}) {
  // 1. Header line (e.g. "path/to/file.tsx (content_hash: ...)")
  if (/\(content_hash:\s*[a-f0-9]+\)/i.test(line)) {
    return (
      <div className="whitespace-pre font-mono text-neutral-500 font-medium pb-1.5 mb-1 border-b border-neutral-200">
        {line}
      </div>
    );
  }

  // 2. Line number prefix like "   5 | " or " 12 │ " or " 1: "
  const numMatch = line.match(/^(\s*\d+\s*[│|:]\s?)(.*)$/);
  if (numMatch) {
    const prefix = numMatch[1];
    const code = numMatch[2];
    const highlighted = highlightCodeToHtml(code, lang);
    return (
      <div className="whitespace-pre font-mono leading-5 hover:bg-[color-mix(in_srgb,var(--husk-n200)_40%,transparent)] px-1 -mx-1 rounded-xs transition-colors">
        <span className="text-neutral-500 select-none mr-2 inline-block min-w-[2.5rem] text-right font-mono">{prefix}</span>
        <span dangerouslySetInnerHTML={{ __html: highlighted }} />
      </div>
    );
  }

  // 3. Truncation notice
  if (/^[….]*\s*\[truncated\s*—.*\]\s*[….]*$/i.test(line)) {
    return (
      <div className="whitespace-pre font-mono text-amber-600 dark:text-amber-400/90 font-medium pt-1.5 mt-1 border-t border-neutral-200">
        {line}
      </div>
    );
  }

  // 4. Regular code / text line
  const highlighted = highlightCodeToHtml(line, lang);
  return (
    <div className="whitespace-pre font-mono leading-5 hover:bg-[color-mix(in_srgb,var(--husk-n200)_40%,transparent)] px-1 -mx-1 rounded-xs transition-colors">
      <span dangerouslySetInnerHTML={{ __html: highlighted }} />
    </div>
  );
});

/** ANSI strip + newline normalization — the cleanup passes run once per
 *  text change (memoized on detailText), NOT once per reveal click. */
function normalizeDetailText(detailText: string): string {
  // 1. Strip ANSI escape codes (terminal color/style sequences that cause garbled text)
  let clean = detailText.replace(/\u001b\[[0-9;]*[a-zA-Z]/g, "");

  // 2. Normalize escaped newlines (e.g. from JSON serialization or double-encoding) and carriage returns
  if (clean.includes("\\n")) {
    clean = clean.replace(/\\r\\n/g, "\n").replace(/\\n/g, "\n");
  }
  clean = clean.replace(/\r\n/g, "\n").replace(/\r/g, "\n");

  // 3. Fix squashed output where newlines were collapsed into spaces before line numbers:
  // e.g. "path (content_hash: ...) 5 | func... 13 | func..." -> "...\n 5 | func...\n 13 | func..."
  clean = clean.replace(/([^\n])\s+(\d+\s*[│|:]\s?)/g, "$1\n$2");
  return clean;
}


function asChipText(value: unknown): string {
  if (typeof value === "string") return value;
  if (value instanceof Date && !Number.isNaN(value.getTime())) {
    return `${value.getFullYear()}年${value.getMonth() + 1}月${value.getDate()}日`;
  }
  return String(value ?? "");
}

function formatChipArgs(label: string, chip: string): string {
  if (!chip) return "";
  let text = chip;
  if (label === "apply_patch") {
    const files: string[] = [];
    const re = /(?:\*\*\*\s*(?:Add|Update|Delete)\s*File:\s*|^(?:added|updated|deleted)\s+)([^\s\n\r]+)/gim;
    let match: RegExpExecArray | null;
    while ((match = re.exec(chip)) !== null) {
      if (match[1] && !files.includes(match[1])) {
        files.push(match[1]);
      }
    }
    if (files.length > 0) {
      return files.join(", ");
    }
  } else if (label === "todo") {
    try {
      const parsed = JSON.parse(chip);
      if (parsed.action) {
        if (parsed.action === "add" && parsed.text) return `add: ${parsed.text}`;
        if (parsed.id) return `${parsed.action}: #${parsed.id}`;
        return parsed.action;
      }
    } catch {
      return chip;
    }
  } else {
    try {
      const parsed = JSON.parse(chip);
      if (typeof parsed === "string") {
        text = parsed;
      } else if (typeof parsed === "object" && parsed !== null) {
        const candidate =
          parsed.path ||
          parsed.file ||
          parsed.file_path ||
          parsed.command ||
          parsed.cmd ||
          parsed.pattern ||
          parsed.query ||
          parsed.action ||
          parsed.target ||
          parsed.url;
        if (typeof candidate === "string") {
          text = candidate;
        } else if (typeof candidate === "number") {
          text = String(candidate);
        } else {
          for (const val of Object.values(parsed)) {
            if (typeof val === "string" && val.trim()) {
              text = val;
              break;
            }
          }
        }
      }
    } catch {
      // not JSON, keep as is
    }
  }
  return text.replace(/\s+/g, " ").trim();
}

function truncateArgText(text: string, maxLen = 60): string {
  if (!text) return "";
  const clean = text.replace(/\s+/g, " ").trim();
  if (clean.length <= maxLen) return clean;
  const sliced = clean.slice(0, maxLen).trimEnd();
  if (sliced.endsWith("...") || sliced.endsWith("…")) {
    return sliced;
  }
  return `${sliced}...`;
}

function isDiffText(text: string): boolean {
  if (!text) return false;
  return (
    (text.includes("--- ") && text.includes("+++ ")) ||
    text.includes("@@ -") ||
    text.startsWith("*** Begin Patch") ||
    text.includes("diff --git ") ||
    (text.includes("added ") && text.includes("@@")) ||
    (text.includes("updated ") && text.includes("@@")) ||
    (text.includes("deleted ") && text.includes("@@"))
  );
}

export type ToolChipRow = {
  /** Calls emitted inside this one (`batch_execute` items) — rendered
   * nested under the row, not counted as top-level calls. */
  children?: ToolChipRow[];
  /** A delegated subagent: rendered as an agent card — name, task, prose
   * body — instead of a tool line, because it is a second agent, not a call. */
  kind?: "agent";
  /** On an agent card: the id a `SteerChild` names to reach this child. */
  childId?: number;
  /** On an agent card: it parked on a question for the parent agent. */
  waiting?: string;
  /** On an agent card: its body in arrival order — reasoning, text and tool
   *  runs (by index into `children`) interleaved, like a turn's own steps. */
  segments?: ChildSegment[];
  chip: string;
  detail?: string[];
  id: string;
  label: string;
  status: "aborted" | "done" | "running" | "rejected";
  uiType?: string;
  approval?: {
    requestId: number;
    diff: string;
    resolved?: boolean;
    approved?: boolean;
    /** Closed by the timeout rather than a verdict — the row says 审批超时. */
    timedOut?: boolean;
  };
};

/** Keep the phase header on an active call, including a running batch item. */
export function toolActivityPreview(rows: ToolChipRow[]): string {
  const row = rows.find((item) => item.status === "running") ?? rows[rows.length - 1];
  if (!row) return "";
  if (row.children?.some((item) => item.status === "running")) {
    return toolActivityPreview(row.children);
  }
  const args = formatChipArgs(row.label, row.chip);
  return args ? `${row.label} · ${args}` : row.label;
}

function DetailLines({
  detailText,
  label,
  chip,
}: {
  detailText: string;
  label: string;
  chip: string;
}) {
  // Progressive reveal: `revealed` is a COUNT, not an expand-all boolean —
  // each click mounts at most EXPAND_STEP new lines, so a huge output
  // never freezes on the "show everything" frame. Language + split memoize
  // on the raw text; per-line memoization means re-renders only pay for
  // the newly revealed lines.
  const [revealed, setRevealed] = useState(MAX_DETAIL_LINES);
  const clean = useMemo(() => normalizeDetailText(detailText), [detailText]);
  const lines = useMemo(() => clean.split("\n"), [clean]);
  const lang = useMemo(
    () => getToolLanguage(clean, label, chip),
    [clean, label, chip],
  );
  const lineCount = lines.length;
  const shownLines = Math.min(revealed, lineCount);
  const hidden = lineCount - shownLines;
  if (!clean) return null;
  return (
    <>
      {lines.slice(0, shownLines).map((line, idx) => (
        <HighlightedLine key={idx} line={line} lang={lang} />
      ))}
      {hidden > 0 && (
        <button
          type="button"
          onClick={() => setRevealed((r) => r + EXPAND_STEP)}
          className="w-full text-center font-mono text-[11px] text-neutral-500 hover:text-neutral-600 pt-1.5 mt-1 border-t border-neutral-200 select-none"
        >
          … 还有 {hidden} 行，点击再展开 {Math.min(EXPAND_STEP, hidden)} 行
        </button>
      )}
    </>
  );
}

export function ToolChips({
  rows,
  renderProse,
}: {
  rows: ToolChipRow[];
  /** Markdown renderer for an agent card's body (the child's streamed text and
   * its final report read as prose, not as tool output). */
  renderProse?: (text: string) => React.ReactNode;
}) {
  const [openRows, setOpenRows] = useState<Set<string>>(new Set());
  const [closedRows, setClosedRows] = useState<Set<string>>(new Set());

  const isRowOpen = (row: ToolChipRow) => {
    if (closedRows.has(row.id)) return false;
    if (openRows.has(row.id)) return true;
    if (row.approval && !row.approval.resolved) return true;
    if (row.label === "todo" || row.uiType === "todo") return true;
    return false;
  };

  const toggleRow = (id: string, currentlyOpen: boolean) => {
    if (currentlyOpen) {
      setOpenRows((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
      setClosedRows((prev) => new Set(prev).add(id));
    } else {
      setClosedRows((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
      setOpenRows((prev) => new Set(prev).add(id));
    }
  };

  if (rows.length === 0) return null;

  const renderRow = (row: ToolChipRow, depth = 0) => {
    if (row.kind === "agent") {
      return (
        <AgentCard
          key={row.id}
          row={row}
          renderProse={renderProse}
          renderRow={(c) => renderRow(c, 1)}
        />
      );
    }

    const rowOpen = isRowOpen(row);
    // A timed-out card was closed by nobody's verdict — it must not read as a
    // plain 已中断, so the timeout is checked before the resolved branches.
    const statusText = row.approval?.timedOut
      ? "审批超时"
      : row.approval && !row.approval.resolved
        ? "待批准"
        : row.status === "rejected"
          ? "已拒绝"
          : row.status === "aborted" || (row.approval?.resolved && !row.approval?.approved)
            ? "已中断"
            : row.status === "running"
              ? "进行中"
              : "完成";

    const detailText = row.detail ? row.detail.join("\n").trim() : "";
    const isDiff =
      row.uiType === "diff" ||
      Boolean(row.approval?.diff) ||
      isDiffText(detailText);

    const diffContent =
      isDiff && detailText.length > 0
        ? detailText
        : (row.approval?.diff ?? (isDiff ? detailText : null));

    const isTodo =
      row.label === "todo" ||
      row.uiType === "todo" ||
      (detailText.includes("Todo List:") && detailText.includes("[")) ||
      /\[[ xX]\]\s*#?\d+/i.test(detailText);

    const hasDetail = Boolean(diffContent) || detailText.length > 0;
    const fullChipText = asChipText(formatChipArgs(row.label, row.chip));
    const displayChipText = truncateArgText(fullChipText, 60);

    return (
      <div key={row.id} className="w-full">
        <button
          aria-expanded={rowOpen}
          className={cn(
            "hover:bg-neutral-100 flex h-7 w-fit max-w-full items-center gap-2 rounded-md px-1.5 text-left select-none transition-colors group",
            hasDetail ? "cursor-pointer" : "cursor-default"
          )}
          onClick={() => {
            if (hasDetail) toggleRow(row.id, rowOpen);
          }}
          type="button"
        >
          <span className="text-neutral-500 group-hover:text-neutral-700 shrink-0 text-xs font-medium transition-colors">
            {asChipText(row.label)}
          </span>
          {displayChipText ? (
            <TooltipSimple
              content={
                <div className="max-w-xl max-h-60 overflow-y-auto font-mono text-xs break-all whitespace-pre-wrap select-text leading-relaxed">
                  {fullChipText}
                </div>
              }
              side="top"
              sideOffset={6}
              className="max-w-xl p-2.5 shadow-xl select-text"
            >
              <span
                className="bg-neutral-100 text-neutral-600 inline-flex h-5 max-w-[280px] sm:max-w-[360px] md:max-w-[420px] items-center rounded-md px-1.5 font-mono text-[11px] shrink min-w-0 cursor-pointer"
              >
                <span className="truncate">{displayChipText}</span>
              </span>
            </TooltipSimple>
          ) : null}
          <span
            className={cn(
              "shrink-0 text-[11px]",
              row.approval?.timedOut ? "text-amber-600 dark:text-amber-400" : "text-neutral-500",
            )}
          >
            {statusText}
            {row.children && row.children.length > 0 ? ` · ${row.children.length} 项` : ""}
          </span>
          {hasDetail && (
            <ChevronDown
              className={cn(
                "h-3 w-3 text-neutral-500 transition-transform duration-250 ease-out shrink-0",
                rowOpen ? "rotate-0" : "-rotate-90"
              )}
            />
          )}
        </button>

        {hasDetail ? (
          <RowDetail open={rowOpen}>
            {diffContent ? (
              <DiffView diff={diffContent} maxHeight={600} />
            ) : isTodo ? (
              <TodoView content={detailText} />
            ) : (
              <div className="w-full rounded-xl border border-neutral-200 bg-neutral-50 overflow-hidden shadow-2xs">
                <div className="p-3 font-mono text-[11.5px] text-neutral-700 leading-relaxed overflow-x-auto max-h-[400px] overflow-y-auto select-text">
                  <DetailLines detailText={detailText} label={row.label} chip={row.chip} />
                </div>
              </div>
            )}
          </RowDetail>
        ) : null}

        {/* Nested batch items — indented under the parent capsule,
            same row UI, not counted as top-level calls. */}
        {row.children && row.children.length > 0 ? (
          <div className="ml-4 pl-2.5 border-l border-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] flex flex-col gap-1 mt-1">
            {row.children.map((c) => renderRow(c, depth + 1))}
          </div>
        ) : null}
      </div>
    );
  };

  /* The collapse header lives on the status row above (`AssistantStatus`
   *  owns open state + the "N 次工具调用" label) — this is just the rows. */
  return (
    <div className="w-full pt-1.5 flex flex-col gap-1.5">
      {rows.map((row) => renderRow(row))}
    </div>
  );
}
