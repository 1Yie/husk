import { memo, useEffect, useMemo, useState } from "react";
import { ChevronDown } from "@keyline-icons/react";
import { Bot, Clock, CornerDownRight, FileText, MessageSquare } from "lucide-react";
import { AssistantStatus } from "@/features/chat/components/assistant-status/index";
import { Orb } from "@/features/chat/components/agent-orb/index";
import { DiffView } from "@/features/chat/components/diff-view/index";
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
  /** On an agent card: the child's streamed reasoning, drawn as its own 思考
   *  block — separate from its answer prose. */
  reasoning?: string;
  /** On an agent card: it parked on a question for the parent agent. */
  waiting?: string;
  /** On an agent card: its body in arrival order — text runs and tool runs
   * (by index into `children`) interleaved, like a session's own steps. */
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

/** Side notes a child's body carries — a steer it received, or the parent's
 *  answer to it. They arrive as blockquote markdown (`> 主代理回复：…`) and
 *  must not read as the child's own answer. */
const NOTE_LABELS = ["用户插话", "主代理回复"] as const;

/** One muted note run, drawn as compact rows instead of markdown: icon, the
 *  speaker as a small label, then the text — so a steer stays a remark and
 *  never blends into the answer prose around it. */
function MutedNote({ text }: { text: string }) {
  const lines = text
    .split("\n")
    .map((line) => line.replace(/^\s*>\s?/, "").trim())
    .filter(Boolean);
  if (lines.length === 0) return null;
  return (
    <div className="flex flex-col gap-1">
      {lines.map((line, i) => {
        const label = NOTE_LABELS.find((name) => line.startsWith(`${name}：`));
        const rest = label ? line.slice(label.length + 1).trim() : line;
        const Icon = label === "用户插话" ? MessageSquare : CornerDownRight;
        return (
          <div
            key={i}
            className="flex items-start gap-1.5 text-[11.5px] leading-relaxed text-neutral-500 select-text"
          >
            <Icon className="mt-[3px] h-3 w-3 shrink-0 text-neutral-400" />
            {label ? <span className="shrink-0 font-medium">{label}</span> : null}
            <span className="min-w-0 whitespace-pre-wrap break-words">{rest}</span>
          </div>
        );
      })}
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
    const body = row.detail ? row.detail.join("\n").trim() : "";
    const running = row.status === "running";
    const waiting = row.waiting != null;
    // A failed child leaves a body behind; an aborted one usually never
    // produced any — the difference is what the header names.
    const state = waiting
      ? "waiting"
      : row.status === "aborted"
        ? body && body !== "已中断"
          ? "failed"
          : "aborted"
        : running
          ? "running"
          : "done";
    const statusText =
      state === "waiting"
        ? "等待回复"
        : state === "failed"
          ? "失败"
          : state === "aborted"
            ? "已中断"
            : state === "running"
              ? "运行中"
              : "完成";
    // Same state, second reading — a dot for the glance, the word for the read.
    const tone =
      state === "waiting"
        ? { text: "text-amber-600 dark:text-amber-400", dot: "bg-amber-500" }
        : state === "failed"
          ? { text: "text-rose-600 dark:text-rose-400", dot: "bg-rose-500" }
          : state === "aborted"
            ? { text: "text-neutral-400", dot: "bg-neutral-300" }
            : state === "running"
              ? { text: "text-neutral-500", dot: "bg-violet-500" }
              : { text: "text-neutral-500", dot: "bg-emerald-500" };
    const lastText = (() => {
      const segs = row.segments ?? [];
      for (let i = segs.length - 1; i >= 0; i--) {
        const s = segs[i];
        if (s.kind === "text" && !s.muted) return s.text.trim();
      }
      return "";
    })();
    // The report is the child's own answer, drawn under its body whenever the
    // body was streamed live — the two are separate deliverables.
    const report = !running && body && body !== lastText ? body : "";
    /** Answer prose — one type size for every run of the card. */
    const prose = (text: string, key?: string) => (
      <div key={key} className="text-[12.5px] leading-relaxed text-neutral-700 select-text">
        {renderProse ? renderProse(text) : <div className="whitespace-pre-wrap">{text}</div>}
      </div>
    );
    /** The child's calls under one rule, so a run of them reads as a block
     *  between prose paragraphs instead of loose rows in it. */
    const toolGroup = (children: ToolChipRow[], key: string) => (
      <div
        key={key}
        role="group"
        aria-label="工具调用"
        className="flex flex-col gap-1 border-l border-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] pl-2.5"
      >
        {children.map((c) => renderRow(c, 1))}
      </div>
    );
    // The child's body in arrival order — consecutive tool segments collapse
    // into ONE group, so a phase of calls sits between two paragraphs.
    const segments: ChildSegment[] =
      row.segments && row.segments.length > 0
        ? row.segments.some((s) => s.kind === "text") || !row.detail?.length
          ? row.segments
          : // A finished child whose report was never streamed as text: the
            // report is its last text run, so it is not dropped.
            [...row.segments, { kind: "text" as const, text: body }]
        : [];
    const blocks: React.ReactNode[] = [];
    let toolRun: ToolChipRow[] = [];
    const flushTools = () => {
      if (toolRun.length === 0) return;
      blocks.push(toolGroup(toolRun, `tools-${toolRun[0].id}`));
      toolRun = [];
    };
    segments.forEach((seg, i) => {
      if (seg.kind === "tool") {
        const child = row.children?.[seg.index];
        if (child) toolRun.push(child);
        return;
      }
      flushTools();
      blocks.push(
        seg.muted ? <MutedNote key={`note-${i}`} text={seg.text} /> : prose(seg.text, `text-${i}`),
      );
    });
    flushTools();
    return (
      <div key={row.id} className="w-full">
        <div className="w-full overflow-hidden rounded-xl border border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)] bg-[color-mix(in_srgb,var(--husk-n100)_45%,transparent)]">
          <button
            aria-expanded={rowOpen}
            className="flex w-full items-center gap-2 px-2.5 py-2 text-left transition-colors hover:bg-[color-mix(in_srgb,var(--husk-n200)_45%,transparent)]"
            onClick={() => toggleRow(row.id, rowOpen)}
            type="button"
          >
            {running ? (
              // The violet orb marks a child still out, as the 子代理 dock does;
              // a settled card falls back to the plain agent mark.
              <Orb
                variant="G2"
                size={14}
                className="shrink-0 text-violet-600 dark:text-violet-400"
              />
            ) : (
              <Bot className="h-3.5 w-3.5 shrink-0 text-neutral-500" />
            )}
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
            <span className="flex shrink-0 items-center gap-1.5 text-[11px]">
              <span aria-hidden="true" className={cn("h-1.5 w-1.5 shrink-0 rounded-full", tone.dot)} />
              <span className={tone.text}>
                {statusText}
                {row.children && row.children.length > 0 ? ` · ${row.children.length} 项` : ""}
              </span>
            </span>
            <ChevronDown
              className={cn(
                "h-3 w-3 text-neutral-500 transition-transform duration-250 ease-out shrink-0",
                rowOpen ? "rotate-0" : "-rotate-90"
              )}
            />
          </button>
          <RowDetail open={rowOpen}>
            {/* One vertical rhythm for the whole card: prose 12.5px, meta 11px. */}
            <div className="flex flex-col gap-2.5 px-2.5 pb-2.5">
              {row.reasoning ? (
                // The main chat's own thinking row, not a second design: live
                // 「思考中」 while the child streams, 「思考过程」 once it settles.
                <AssistantStatus
                  mode={running ? "thinking" : "thought"}
                  thinkingText={row.reasoning}
                />
              ) : null}
              {segments.length > 0 ? (
                blocks
              ) : (
                <>
                  {body ? (
                    prose(body)
                  ) : running ? (
                    <div className="text-[11px] text-neutral-500">等待子代理输出…</div>
                  ) : null}
                  {row.children && row.children.length > 0
                    ? toolGroup(row.children, `children-${row.id}`)
                    : null}
                </>
              )}
              {report ? (
                // The child's final report, below whatever it streamed — the
                // body is its working trace, this is its deliverable.
                <div className="border-t border-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] pt-2">
                  <div className="mb-1 flex items-center gap-1 text-[11px] text-neutral-500">
                    <FileText className="h-3.5 w-3.5 shrink-0" />
                    报告
                  </div>
                  {prose(report)}
                </div>
              ) : null}
              {waiting ? (
                // Parked on a question for the parent — the card's current
                // state, so it sits last, under what the child already did.
                <div className="rounded-lg border border-amber-400/50 bg-amber-50/60 px-2.5 py-2 dark:bg-amber-400/10">
                  <div className="flex items-center gap-1.5 text-[11.5px] font-medium text-amber-700 dark:text-amber-300">
                    <Clock className="h-3 w-3 shrink-0" />
                    等待主代理回复
                  </div>
                  {row.waiting ? (
                    <div className="mt-0.5 text-[12px] leading-relaxed text-amber-700/90 whitespace-pre-wrap select-text">
                      {row.waiting}
                    </div>
                  ) : null}
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
