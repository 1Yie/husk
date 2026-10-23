// SubagentsList — the 子代理 view of the session dock: every child the session
// delegated, one card each. A child still out carries its steer box; one that
// ended keeps its verdict and report, so the view outlives the run and comes
// back with the session (the cards are rebuilt from the persisted log) — until
// the reader clears the finished ones, which the store remembers per session.
// The dock owns the frame.

import { useCallback, useRef, useState } from "react";
import { ChevronDown, ChevronRight, CircleCheck, CircleSlash, CircleX } from "lucide-react";
import { cn } from "@/lib/utils";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Orb } from "@/features/chat/components/agent-orb";
import {
  childIsLive,
  childKey,
  type ChildStatus,
  type SessionChild,
} from "@/features/chat/hooks/running-children";
import { CHILD_STATUS_LABEL, WAITING_STEER_HINT } from "@/lib/subagents";
import { useAgentStore } from "@/stores/agent-store";
import { ChildSteerInput } from "./child-steer";

/** One 16px lane for every state, so names line up whatever the glyph. */
function StatusGlyph({ status }: { status: ChildStatus }) {
  return (
    <span className="mt-px flex h-4 w-4 flex-none items-center justify-center">
      {status === "running" || status === "waiting" ? (
        <Orb variant="G2" size={16} className="text-violet-600 dark:text-violet-400" />
      ) : status === "done" ? (
        // A step below the others: a filled check carries more ink.
        <CircleCheck className="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400" />
      ) : status === "failed" ? (
        <CircleX className="h-4 w-4 text-rose-600 dark:text-rose-400" />
      ) : (
        <CircleSlash className="h-3.5 w-3.5 text-neutral-400" />
      )}
    </span>
  );
}

const STATUS_CHIP: Record<ChildStatus, string> = {
  running: "bg-violet-500/10 text-violet-700 dark:text-violet-300",
  waiting: "bg-amber-500/15 text-amber-700 dark:text-amber-300",
  done: "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  failed: "bg-rose-500/10 text-rose-700 dark:text-rose-300",
  aborted: "bg-neutral-500/10 text-neutral-500",
};

/** The task a child was handed: three lines in the card, the whole text in a
 *  hover panel that scrolls. A native `title` cannot carry it — the webview
 *  draws that itself, unclamped, and a long task buries the window under it. */
function TaskText({ text }: { text: string }) {
  const [clipped, setClipped] = useState(false);
  const roRef = useRef<ResizeObserver | null>(null);
  // Keyed to `text`: the box outlives the string, so only a new ref identity
  // makes React detach and re-attach, re-running the measurement.
  const measure = useCallback(
    (el: HTMLDivElement | null) => {
      roRef.current?.disconnect();
      roRef.current = null;
      if (!el || typeof ResizeObserver === "undefined") return;
      // The clamp hides what overflows, so a box shorter than its content is
      // exactly a task that was cut.
      const check = () => setClipped(el.scrollHeight > el.clientHeight + 1);
      check();
      const ro = new ResizeObserver(check);
      ro.observe(el);
      roRef.current = ro;
    },
    [text],
  );

  // The clamped box is the trigger even while nothing opens: a task long
  // enough to need the panel must not remount the card under the pointer.
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div
          ref={measure}
          className="mt-0.5 line-clamp-3 text-[11.5px] leading-relaxed text-neutral-500 dark:text-neutral-400"
        >
          {text}
        </div>
      </TooltipTrigger>
      {clipped && (
        // Scrollable and hoverable: Radix keeps the bubble open while the
        // pointer is over it, so the reader can scroll the rest of the task.
        <TooltipContent side="top" sideOffset={6} className="max-w-[24rem] p-2.5 select-text">
          <div className="max-h-[min(20rem,var(--radix-tooltip-content-available-height))] overflow-y-auto overflow-x-hidden overscroll-contain whitespace-pre-wrap break-words text-left text-xs leading-relaxed select-text">
            {text}
          </div>
        </TooltipContent>
      )}
    </Tooltip>
  );
}

function ChildCard({ child }: { child: SessionChild }) {
  const live = childIsLive(child);
  const [reportOpen, setReportOpen] = useState(false);
  return (
    <div
      data-child-status={child.status}
      className={cn(
        "flex w-full flex-col gap-2 rounded-[14px] border border-[color-mix(in_srgb,var(--husk-n200)_90%,transparent)] px-3 py-2.5",
        // A finished card steps back so the ones still out read first.
        live
          ? "bg-white shadow-[0_2px_8px_rgba(0,0,0,0.04)] dark:bg-[var(--husk-active)]"
          : "bg-[color-mix(in_srgb,var(--husk-n50)_60%,transparent)]",
      )}
    >
      <div className="flex items-start gap-2">
        <StatusGlyph status={child.status} />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <span className="min-w-0 truncate font-mono text-[11.5px] text-neutral-800" title={child.name}>
              {child.name}
            </span>
            <span
              className={cn(
                "flex-none rounded-full px-1.5 py-px text-[10px] font-medium",
                STATUS_CHIP[child.status],
              )}
            >
              {CHILD_STATUS_LABEL[child.status]}
            </span>
          </div>
          {/* The task is what a steer answers: three lines here, the rest in
              the hover panel. */}
          {child.task && <TaskText text={child.task} />}
        </div>
      </div>
      {/* A parked child takes the text only after it resumes — saying so beats
          a steer that looks swallowed. `ml-6` clears the glyph lane. */}
      {child.status === "waiting" && (
        <div className="ml-6 flex flex-col gap-1">
          <div className="rounded-[10px] bg-[color-mix(in_srgb,var(--husk-n100)_80%,transparent)] px-2.5 py-1.5 text-[11.5px] leading-relaxed text-neutral-700 select-text dark:text-neutral-300">
            {child.waiting}
          </div>
          <div className="text-[10.5px] leading-relaxed text-amber-600 dark:text-amber-400">
            {WAITING_STEER_HINT}
          </div>
        </div>
      )}
      {live && child.childId != null && (
        <div className="ml-6">
          <ChildSteerInput childId={child.childId} />
        </div>
      )}
      {!live && child.report && (
        <div className="ml-6">
          <button
            type="button"
            aria-expanded={reportOpen}
            onClick={() => setReportOpen((o) => !o)}
            className="flex items-center gap-1 rounded-full text-[11px] text-neutral-500 transition-colors hover:text-neutral-800"
          >
            {reportOpen ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
            报告
          </button>
          {reportOpen && (
            <div className="mt-1.5 max-h-64 overflow-y-auto rounded-[10px] bg-[color-mix(in_srgb,var(--husk-n100)_80%,transparent)] px-2.5 py-2 text-[11.5px] leading-relaxed whitespace-pre-wrap break-words text-neutral-700 select-text dark:text-neutral-300">
              {child.report}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

/** The 子代理 view's body: children still out first, in the order they were
 *  sent; the finished ones after them, newest first. */
export function SubagentsList({
  children,
  root,
  session,
}: {
  children: SessionChild[];
  /** Which session the clear addresses — the cleared set lives in the store,
   *  so a child does not come back on the next event or the next reopen. */
  root: string;
  session: number;
}) {
  const dismiss = useAgentStore((s) => s.dismissChildren);
  const live = children.filter(childIsLive);
  const ended = children.filter((c) => !childIsLive(c)).reverse();
  return (
    <div className="flex w-full flex-col gap-2">
      {/* Same row and wording as 后台's clear: one gesture for "drop what is
          done", wherever the reader is. The ones still out are untouched. */}
      {ended.length > 0 && (
        <div className="flex justify-end px-2">
          <button
            type="button"
            onClick={() => dismiss(root, session, ended.map(childKey))}
            className="rounded-full px-2 py-0.5 text-[11px] text-neutral-500 transition-colors hover:bg-[color-mix(in_srgb,var(--husk-black)_5%,transparent)] hover:text-neutral-800"
          >
            清除已结束
          </button>
        </div>
      )}
      {live.map((c, i) => (
        <ChildCard key={c.childId ?? `live-${i}`} child={c} />
      ))}
      {ended.length > 0 && live.length > 0 && (
        <div className="px-1 pt-1 text-[11px] font-medium text-neutral-400">已结束</div>
      )}
      {ended.map((c, i) => (
        <ChildCard key={c.childId ?? `ended-${i}`} child={c} />
      ))}
    </div>
  );
}
