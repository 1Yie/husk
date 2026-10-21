// JobsList — the 后台 tab body of the session dock: the session's background
// jobs (`bg_run`), newest first, with their controls and an on-demand log tail.
// Fed by `SessionView.bgJobs` (`husk:bg_jobs` snapshots); logs and controls go
// straight to the host (`bg_logs` / `bg_control`), outside the command pump, so
// a kill lands while a turn is running. The dock owns the frame.

import { useEffect, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  CircleCheck,
  CirclePause,
  CircleX,
  Pause,
  Play,
  Square,
  X,
} from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { TooltipSimple } from "@/components/ui/tooltip";
import { Orb } from "@/features/chat/components/agent-orb";
import type { BgJob } from "@/features/chat/hooks/stream-view";
import { bgJobLogs, controlBgJob, type BgJobAction } from "@/lib/agent-ipc/sessions";
import { formatElapsed, jobElapsedMs, jobStateLabel, jobTone, type JobTone } from "@/lib/bg-jobs";

/** How often an expanded running job's log tail is re-read. */
const LOG_POLL_MS = 2000;

function ToneIcon({ tone }: { tone: JobTone }) {
  const cls = "h-3.5 w-3.5 flex-none";
  switch (tone) {
    case "running":
      // B4 in the row, B2 in the pill: the panel shows one job at a time and
      // gets the denser glyph; the pill speaks for the whole session.
      return <Orb variant="B4" size={14} className="flex-none text-sky-600 dark:text-sky-400" />;
    case "paused":
      return <CirclePause className={cn(cls, "text-neutral-400")} />;
    case "ok":
      // One step below the other state glyphs, for the same reason as the
      // pill's check: the filled circle reads heavier at an equal size.
      return <CircleCheck className="h-3 w-3 flex-none text-emerald-600 dark:text-emerald-400" />;
    case "failed":
      return <CircleX className={cn(cls, "text-rose-600 dark:text-rose-400")} />;
    default:
      return <Square className={cn(cls, "text-amber-600 dark:text-amber-400")} />;
  }
}

/** One second of wall clock, while anything is running — the rows tick their
 *  clocks from the snapshot instead of asking the host every second. */
function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [active]);
  return now;
}

/** The job's log tail while the row is open — re-read on a timer only while
 *  the job still runs; a finished log does not change. */
function useLogTail(root: string, session: number, job: BgJob, open: boolean) {
  const [tail, setTail] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const running = job.state.kind === "running";
  // A job that just finished gets one last read: its final lines are the ones
  // that say why.
  const settled = job.state.kind === "done" ? job.state.label : "";
  useEffect(() => {
    if (!open) return;
    let live = true;
    const read = () =>
      void bgJobLogs(root, session, job.id)
        .then((t) => {
          if (!live) return;
          setTail(t);
          setError(null);
        })
        .catch((e) => live && setError(String(e)));
    read();
    const t = running ? setInterval(read, LOG_POLL_MS) : null;
    return () => {
      live = false;
      if (t) clearInterval(t);
    };
  }, [root, session, job.id, open, running, settled]);
  return { tail, error };
}

function ActionButton({
  label,
  onClick,
  disabled,
  children,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <TooltipSimple content={label} side="top">
      <button
        type="button"
        aria-label={label}
        disabled={disabled}
        onClick={(e) => {
          e.stopPropagation();
          onClick();
        }}
        className="flex h-5 w-5 flex-none items-center justify-center rounded text-neutral-500 transition-colors hover:bg-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] hover:text-neutral-800 disabled:opacity-40"
      >
        {children}
      </button>
    </TooltipSimple>
  );
}

function JobRow({
  job,
  root,
  session,
  now,
  snapshotAt,
  open,
  onToggle,
  onJobs,
}: {
  job: BgJob;
  root: string;
  session: number;
  now: number;
  snapshotAt: number;
  open: boolean;
  onToggle: () => void;
  onJobs: (jobs: BgJob[]) => void;
}) {
  const tone = jobTone(job);
  // A restored row is a record, not a run: the host kept the list, not the
  // process. It still reads, but nothing here can be acted on or tailed.
  const restored = job.restored === true;
  const [busy, setBusy] = useState(false);
  const { tail, error } = useLogTail(root, session, job, open && !restored);
  const act = (action: BgJobAction) => {
    setBusy(true);
    void controlBgJob(root, session, job.id, action)
      .then((r) => onJobs(r.jobs))
      .catch((e) => toast.error(`操作失败：${e}`))
      .finally(() => setBusy(false));
  };
  const live = job.state.kind !== "done";
  return (
    <div className="w-full">
      <div
        role="button"
        tabIndex={0}
        aria-expanded={open}
        aria-label={`${job.id} ${jobStateLabel(job)}`}
        onClick={onToggle}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            onToggle();
          }
        }}
        className="group flex w-full cursor-pointer items-center gap-1.5 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-[color-mix(in_srgb,var(--husk-n200)_45%,transparent)]"
      >
        <span className="flex-none text-neutral-500">
          {open ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
        </span>
        <ToneIcon tone={tone} />
        <span className="min-w-0 flex-1 truncate font-mono text-[11.5px] text-neutral-700" title={job.command}>
          {job.command}
        </span>
        {!job.has_own_sink && (
          <span className="flex-none rounded bg-neutral-500/10 px-1 py-px text-[9.5px] text-neutral-500">
            子代理
          </span>
        )}
        {/* Controls replace the clock on hover, so the row keeps one width. */}
        <span className="flex flex-none items-center gap-0.5 group-hover:hidden group-focus-within:hidden">
          <span
            className={cn(
              "font-mono text-[10.5px] tabular-nums",
              tone === "failed" ? "text-rose-600 dark:text-rose-400" : "text-neutral-400",
            )}
          >
            {live ? formatElapsed(jobElapsedMs(job, snapshotAt, now)) : jobStateLabel(job)}
          </span>
        </span>
        <span className="hidden flex-none items-center gap-0.5 group-hover:flex group-focus-within:flex">
          {restored ? (
            // The record has no process behind it — the only thing to do with
            // it is drop it from the list.
            <ActionButton label="移除记录" disabled={busy} onClick={() => act("remove")}>
              <X className="h-3 w-3" />
            </ActionButton>
          ) : (
            <>
              {job.state.kind === "running" && (
                <ActionButton label="暂停" disabled={busy} onClick={() => act("pause")}>
                  <Pause className="h-3 w-3" />
                </ActionButton>
              )}
              {job.state.kind === "paused" && (
                <ActionButton label="继续" disabled={busy} onClick={() => act("resume")}>
                  <Play className="h-3 w-3" />
                </ActionButton>
              )}
              {live ? (
                <ActionButton label="终止" disabled={busy} onClick={() => act("kill")}>
                  <Square className="h-3 w-3" />
                </ActionButton>
              ) : (
                <ActionButton label="移除" disabled={busy} onClick={() => act("remove")}>
                  <X className="h-3 w-3" />
                </ActionButton>
              )}
            </>
          )}
        </span>
      </div>
      {open && (
        <div className="mb-1.5 ml-6 mr-1">
          <div className="mb-1 flex items-center gap-2 text-[10.5px] text-neutral-400">
            <span className="font-mono">{job.id}</span>
            <span>{jobStateLabel(job)}</span>
            <span className="font-mono tabular-nums">{formatElapsed(jobElapsedMs(job, snapshotAt, now))}</span>
          </div>
          <pre className="max-h-64 overflow-auto rounded-md border border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)] bg-[color-mix(in_srgb,var(--husk-n100)_70%,transparent)] px-2 py-1.5 font-mono text-[10.5px] leading-[1.45] whitespace-pre-wrap break-all text-neutral-600 select-text">
            {restored
              ? "日志随任务进程一起丢失"
              : error
                ? `读取日志失败：${error}`
                : tail === null
                  ? "读取中…"
                  : tail.trim() || "（暂无输出）"}
          </pre>
        </div>
      )}
    </div>
  );
}

/** The 后台 view's body. `onJobs` takes the list a control call returned, so
 *  the row updates before the next snapshot arrives. */
export function JobsList({
  jobs,
  snapshotAt,
  root,
  session,
  onJobs,
}: {
  jobs: BgJob[];
  snapshotAt: number;
  root: string;
  session: number;
  onJobs: (jobs: BgJob[]) => void;
}) {
  const [opened, setOpened] = useState<Set<string>>(new Set());
  const now = useNow(jobs.some((j) => j.state.kind === "running"));
  // Newest first: the job that just started is the one being asked about.
  const ordered = [...jobs].reverse();
  // A restored record is finished too, and the host can drop it from the file
  // — so "清除已结束" covers both kinds of ended row.
  const finished = jobs.filter((j) => j.state.kind === "done");
  const clearFinished = () => {
    void Promise.allSettled(finished.map((j) => controlBgJob(root, session, j.id, "remove"))).then(
      (results) => {
        const last = results.reverse().find((r) => r.status === "fulfilled");
        if (last && last.status === "fulfilled") onJobs(last.value.jobs);
        if (results.some((r) => r.status === "rejected")) toast.error("部分任务未能移除");
      },
    );
  };
  const toggle = (id: string) =>
    setOpened((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  return (
    <div className="w-full">
      {finished.length > 0 && (
        <div className="flex justify-end px-2 pb-1">
          <button
            type="button"
            onClick={clearFinished}
            className="rounded px-1.5 py-0.5 text-[11px] text-neutral-500 transition-colors hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] hover:text-neutral-700"
          >
            清除已结束
          </button>
        </div>
      )}
      {ordered.map((job) => (
        <JobRow
          key={job.id}
          job={job}
          root={root}
          session={session}
          now={now}
          snapshotAt={snapshotAt}
          open={opened.has(job.id)}
          onToggle={() => toggle(job.id)}
          onJobs={onJobs}
        />
      ))}
    </div>
  );
}
