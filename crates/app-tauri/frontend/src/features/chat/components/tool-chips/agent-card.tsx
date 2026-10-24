// The subagent card: a delegated child drawn as a second agent — its name, the
// task it was handed, then its body. The chat card and the sidebar's 工作过程
// share the body (`AgentTranscript`), so a child reads the same in both places.

import { useState } from "react";
import { ChevronDown } from "@keyline-icons/react";
import { Bot, Clock, CornerDownRight, FileText, MessageSquare } from "lucide-react";
import { Orb } from "@/features/chat/components/agent-orb/index";
import { ThinkingBlock } from "@/features/chat/components/chat-stream/turn-blocks";
import type { ToolChipRow } from "@/features/chat/components/tool-chips/index";
import { RowDetail } from "@/features/chat/components/tool-chips/row-detail";
import { reportBody } from "@/features/chat/hooks/report-text";
import type { ChildSegment } from "@/features/chat/hooks/stream-view";
import { TooltipSimple } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

/** The body a card carries as one string — the raw tool result, streamed or final. */
function bodyOf(row: ToolChipRow): string {
  return row.detail ? row.detail.join("\n").trim() : "";
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

/** A child's body in arrival order — reasoning drawn as the main chat's thinking
 *  row, a run of its calls grouped under one rule, its prose, then the report it
 *  ended on and the question it parked on. The chat card and the sidebar share it. */
export function AgentTranscript({
  row,
  renderProse,
  renderRow,
}: {
  row: ToolChipRow;
  /** Markdown renderer for the child's prose (its streamed text and its report
   *  read as prose, not as tool output). */
  renderProse?: (text: string) => React.ReactNode;
  /** Draws one of the child's own calls. */
  renderRow: (row: ToolChipRow) => React.ReactNode;
}) {
  const body = bodyOf(row);
  const running = row.status === "running";
  const waiting = row.waiting != null;
  const lastText = (() => {
    const segs = row.segments ?? [];
    for (let i = segs.length - 1; i >= 0; i--) {
      const s = segs[i];
      if (s.kind === "text" && !s.muted) return s.text.trim();
    }
    return "";
  })();
  // The body is the raw tool result; the kernel's `[subagent report — …]`
  // header (and a batch section marker) is metadata for the parent model, so
  // it never reaches the reader. Stripped here only — `row.detail` keeps it.
  const reportText = reportBody(body);
  // The report is the child's own answer, drawn under its body whenever the
  // body was streamed live — the two are separate deliverables.
  const report = !running && reportText && body !== lastText ? reportText : "";
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
      {children.map((c) => renderRow(c))}
    </div>
  );
  // The child's body in arrival order — consecutive tool segments collapse
  // into ONE group, so a phase of calls sits between two paragraphs, and a
  // thinking run sits exactly where the reasoning happened.
  const segments: ChildSegment[] =
    row.segments && row.segments.length > 0
      ? row.segments.some((s) => s.kind === "text") || !row.detail?.length || !reportText
        ? row.segments
        : // A finished child whose report was never streamed as text: the
          // report is its last text run, so it is not dropped.
          [...row.segments, { kind: "text" as const, text: reportText }]
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
    if (seg.kind === "thinking") {
      // The main turn's own thinking row, drawn where the reasoning happened.
      // A run is still arriving only while nothing followed it.
      blocks.push(
        <ThinkingBlock
          key={`thinking-${i}`}
          text={seg.text}
          done={!running || i < segments.length - 1}
        />,
      );
      return;
    }
    blocks.push(
      seg.muted ? <MutedNote key={`note-${i}`} text={seg.text} /> : prose(seg.text, `text-${i}`),
    );
  });
  flushTools();
  return (
    // One vertical rhythm for the whole card: prose 12.5px, meta 11px.
    <div className="flex flex-col gap-2.5 px-2.5 pb-2.5">
      {segments.length > 0 ? (
        blocks
      ) : (
        <>
          {reportText ? (
            prose(reportText)
          ) : running ? (
            <div className="text-[11px] text-neutral-500">等待子代理输出…</div>
          ) : null}
          {row.children && row.children.length > 0
            ? toolGroup(row.children, `children-${row.id}`)
            : null}
        </>
      )}
      {report ? (
        // The child's final report, below whatever it streamed — the body is its
        // working trace, this is its deliverable.
        <div className="border-t border-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] pt-2">
          <div className="mb-1 flex items-center gap-1 text-[11px] text-neutral-500">
            <FileText className="h-3.5 w-3.5 shrink-0" />
            报告
          </div>
          {prose(report)}
        </div>
      ) : null}
      {waiting ? (
        // Parked on a question for the parent — the card's current state, so it
        // sits last, under what the child already did.
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
  );
}

/** A subagent reads as a small card, not a tool line: agent name, the task
 *  it was handed, then its transcript in arrival order (or its report). The
 *  body is closed until the reader opens it — a child's transcript is long,
 *  and opening it unasked buries the turn that spawned it. */
export function AgentCard({
  row,
  renderProse,
  renderRow,
  defaultOpen = false,
}: {
  row: ToolChipRow;
  renderProse?: (text: string) => React.ReactNode;
  renderRow: (row: ToolChipRow) => React.ReactNode;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const body = bodyOf(row);
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
  return (
    <div className="w-full">
      <div className="w-full overflow-hidden rounded-xl border border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)] bg-[color-mix(in_srgb,var(--husk-n100)_45%,transparent)]">
        <button
          aria-expanded={open}
          className="flex w-full items-center gap-2 px-2.5 py-2 text-left transition-colors hover:bg-[color-mix(in_srgb,var(--husk-n200)_45%,transparent)]"
          onClick={() => setOpen((o) => !o)}
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
              open ? "rotate-0" : "-rotate-90"
            )}
          />
        </button>
        <RowDetail open={open}>
          <AgentTranscript row={row} renderProse={renderProse} renderRow={renderRow} />
        </RowDetail>
      </div>
    </div>
  );
}
