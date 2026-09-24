import { useEffect, useMemo, useState } from "react";
import { ChevronDown } from "@keyline-icons/react";
import { Bot } from "lucide-react";
import { DiffView } from "@/features/chat/components/diff-view/index";
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

function renderHighlightedLines(
  detailText: string,
  label: string,
  chip: string,
  maxLines = Number.MAX_SAFE_INTEGER,
) {
  if (!detailText) return null;

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

  const lang = getToolLanguage(clean, label, chip);
  const lines = clean.split("\n");

  return lines.slice(0, maxLines).map((line, idx) => {
    // 1. Header line (e.g. "path/to/file.tsx (content_hash: ...)")
    if (/\(content_hash:\s*[a-f0-9]+\)/i.test(line)) {
      return (
        <div key={idx} className="whitespace-pre font-mono text-neutral-500 font-medium pb-1.5 mb-1 border-b border-neutral-200">
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
        <div key={idx} className="whitespace-pre font-mono leading-5 hover:bg-[color-mix(in_srgb,var(--husk-n200)_40%,transparent)] px-1 -mx-1 rounded-xs transition-colors">
          <span className="text-neutral-500 select-none mr-2 inline-block min-w-[2.5rem] text-right font-mono">{prefix}</span>
          <span dangerouslySetInnerHTML={{ __html: highlighted }} />
        </div>
      );
    }

    // 3. Truncation notice
    if (/^[….]*\s*\[truncated\s*—.*\]\s*[….]*$/i.test(line)) {
      return (
        <div key={idx} className="whitespace-pre font-mono text-amber-600 dark:text-amber-400/90 font-medium pt-1.5 mt-1 border-t border-neutral-200">
          {line}
        </div>
      );
    }

    // 4. Regular code / text line
    const highlighted = highlightCodeToHtml(line, lang);
    return (
      <div key={idx} className="whitespace-pre font-mono leading-5 hover:bg-[color-mix(in_srgb,var(--husk-n200)_40%,transparent)] px-1 -mx-1 rounded-xs transition-colors">
        <span dangerouslySetInnerHTML={{ __html: highlighted }} />
      </div>
    );
  });
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
  };
};

function DetailLines({
  detailText,
  label,
  chip,
}: {
  detailText: string;
  label: string;
  chip: string;
}) {
  const [expanded, setExpanded] = useState(false);
  // Cheap line count for the footer — the real split happens inside
  // renderHighlightedLines; this is just `\n` accounting.
  const lineCount = useMemo(
    () => detailText.split("\n").length,
    [detailText],
  );
  const capped = !expanded && lineCount > MAX_DETAIL_LINES;
  // Highlighting every line is the expensive part — memoize so a
  // re-render triggered by a sibling (row open/close, parent re-render)
  // doesn't redo it. `expanded` participates via `capped`.
  const highlighted = useMemo(
    () =>
      renderHighlightedLines(
        detailText,
        label,
        chip,
        capped ? MAX_DETAIL_LINES : Number.MAX_SAFE_INTEGER,
      ),
    [detailText, label, chip, capped],
  );
  return (
    <>
      {highlighted}
      {capped && (
        <button
          type="button"
          onClick={() => setExpanded(true)}
          className="w-full text-center font-mono text-[11px] text-neutral-500 hover:text-neutral-600 pt-1.5 mt-1 border-t border-neutral-200 select-none"
        >
          … 还有 {lineCount - MAX_DETAIL_LINES} 行，点击展开全部
        </button>
      )}
    </>
  );
}

/** Detail body that mounts its children only after the row has been
 * opened once — the grid collapse is CSS-only (`0fr` + overflow-hidden),
 * so without this every collapsed row still paid a full DiffView /
 * DetailLines render at mount (hundreds of highlighted lines per
 * tool call — the actual ~50-160ms-per-turn mount cost). Once opened it
 * stays mounted so the collapse animation keeps working. */
function RowDetail({ open, children }: { open: boolean; children: React.ReactNode }) {
  const [everOpened, setEverOpened] = useState(open);
  useEffect(() => {
    if (open) setEverOpened(true);
  }, [open]);
  return (
    <div
      className="grid transition-[grid-template-rows,opacity] duration-250 ease-out w-full"
      style={{
        gridTemplateRows: open ? "1fr" : "0fr",
        opacity: open ? 1 : 0,
      }}
    >
      <div className="min-h-0 overflow-hidden w-full">
        <div className="w-full pt-1 pb-1.5">
          {everOpened ? children : null}
        </div>
      </div>
    </div>
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
    // An agent card is open until the reader closes it: watching a subagent
    // work is the point, and auto-collapsing when it finishes would yank the
    // report away mid-read.
    if (row.kind === "agent") return true;
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

  /** A subagent reads as a small card, not a tool line: agent name, the task
   *  it was handed, then whatever it has streamed so far (or its report). */
  function renderAgentRow(row: ToolChipRow) {
    const rowOpen = isRowOpen(row);
    const statusText =
      row.status === "aborted" ? "已中断" : row.status === "running" ? "进行中" : "完成";
    const body = row.detail ? row.detail.join("\n").trim() : "";
    return (
      <div key={row.id} className="w-full">
        <div className="w-full overflow-hidden rounded-xl border border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)] bg-[color-mix(in_srgb,var(--husk-n100)_45%,transparent)]">
          <button
            aria-expanded={rowOpen}
            className="flex w-full items-center gap-2 px-2.5 py-2 text-left transition-colors hover:bg-[color-mix(in_srgb,var(--husk-n200)_45%,transparent)]"
            onClick={() => toggleRow(row.id, rowOpen)}
            type="button"
          >
            <Bot className="h-3.5 w-3.5 shrink-0 text-neutral-500" />
            <span className="text-neutral-700 shrink-0 text-[12.5px] font-medium">
              {row.label}
            </span>
            {row.chip ? (
              <TooltipSimple
                content={
                  <div className="max-w-xl max-h-60 overflow-y-auto text-xs break-all whitespace-pre-wrap select-text leading-relaxed">
                    {row.chip}
                  </div>
                }
                side="top"
                sideOffset={6}
                className="max-w-xl p-2.5 shadow-xl select-text"
              >
                <span className="text-neutral-500 min-w-0 flex-1 cursor-pointer truncate text-[11.5px]">
                  {row.chip}
                </span>
              </TooltipSimple>
            ) : (
              <span className="flex-1" />
            )}
            <span className="text-neutral-500 shrink-0 text-[11px]">
              {statusText}
              {row.children && row.children.length > 0 ? ` · ${row.children.length} 项` : ""}
            </span>
            <ChevronDown
              className={cn(
                "h-3 w-3 text-neutral-500 transition-transform duration-250 ease-out shrink-0",
                rowOpen ? "rotate-0" : "-rotate-90"
              )}
            />
          </button>
          <RowDetail open={rowOpen}>
            <div className="px-2.5 pb-2.5">
              {body ? (
                <div className="text-neutral-700 text-[12.5px] leading-relaxed select-text">
                  {renderProse ? (
                    renderProse(body)
                  ) : (
                    <div className="whitespace-pre-wrap">{body}</div>
                  )}
                </div>
              ) : row.status === "running" ? (
                <div className="text-neutral-500 text-[11.5px]">等待子代理输出…</div>
              ) : null}
              {row.children && row.children.length > 0 ? (
                <div className="border-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] mt-2 flex flex-col gap-1 border-l pl-2.5">
                  {row.children.map((c) => renderRow(c, 1))}
                </div>
              ) : null}
            </div>
          </RowDetail>
        </div>
      </div>
    );
  }

  const renderRow = (row: ToolChipRow, depth = 0) => {
    if (row.kind === "agent") return renderAgentRow(row);
    const rowOpen = isRowOpen(row);
    const statusText =
      row.approval && !row.approval.resolved
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
          <span className="text-neutral-500 shrink-0 text-[11px]">
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
