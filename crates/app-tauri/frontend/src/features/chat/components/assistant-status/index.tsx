import { memo, useEffect, useState, type ReactNode } from "react";
import { Orb } from "@/features/chat/components/agent-orb/index";
import { ChevronDown } from "@keyline-icons/react";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";
import { ReasoningBody } from "@/features/chat/components/chat-stream/reasoning-body";

/** The live modes — i.e. the states the row is actively working in, as
 *  opposed to the settled "思考过程" recap. */
function liveMode(
  mode: "reply" | "thought" | "thinking" | "tools" | "tools_done" | "compacting" | null
): boolean {
  return (
    mode === "reply" ||
    mode === "tools" ||
    mode === "thinking" ||
    mode === "compacting"
  );
}

/** Duration label — bare seconds under a minute ("20s"), then `m:ss`, and
 *  `h:mm:ss` once an hour in (a stalled tool call can sit there a long while). */
export function formatElapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  if (total < 60) return `${total}s`;
  const mins = Math.floor(total / 60);
  const secs = String(total % 60).padStart(2, "0");
  if (mins < 60) return `${mins}:${secs}`;
  return `${Math.floor(mins / 60)}:${String(mins % 60).padStart(2, "0")}:${secs}`;
}

/** How long a status row has spent in its live modes: `banked` totals the
 *  passes that already ended, `since` anchors the one in flight (`null` once
 *  the row settles).
 *
 *  Time is BANKED across passes rather than restarted on each mode flip: some
 *  backends emit reasoning deltas after answer text, which reopens the block
 *  (`applyEvent`) and flips "思考过程" back to "思考中" mid-turn. A clock
 *  anchored per-pass would blink to 0s every time that happens; this one keeps
 *  counting, so the figure only ever grows. */
interface Trace {
  banked: number;
  since: number | null;
}

/** The row's live time so far. */
function traceMs(trace: Trace, now: number): number {
  return trace.banked + (trace.since == null ? 0 : now - trace.since);
}

/** Move a row's clock onto a new mode. Entering a live mode opens a pass;
 *  leaving one banks it. Returns null for a row that was never live — a
 *  thinking block replayed into an already-settled view measured nothing, so
 *  it must render no duration rather than a fake "· 0s". */
function advanceClock(prev: Trace | null, live: boolean, now: number): Trace | null {
  if (live) return { banked: prev?.banked ?? 0, since: now };
  if (!prev || prev.since == null) return prev;
  return { banked: prev.banked + (now - prev.since), since: null };
}

/** The ticking `· 20s` suffix. Owns the 1s re-render so the header's orbs and
 *  label row don't re-render every second. The value is read straight off the
 *  wall clock — a throttled interval in a background webview then can't leave
 *  a drifted figure behind on wake. `live` false = settled row: the duration
 *  stays put and no timer is kept alive. */
const ElapsedLabel = memo(function ElapsedLabel({
  trace,
  live,
}: {
  trace: Trace;
  live: boolean;
}) {
  const [, tick] = useState(0);
  useEffect(() => {
    if (!live) return;
    const id = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(id);
  }, [live]);
  const ms = traceMs(trace, Date.now());
  // Nothing worth showing during the first second.
  if (ms < 1000) return null;
  return (
    <span className="whitespace-nowrap text-neutral-500 font-normal tabular-nums">
      · {formatElapsed(ms)}
    </span>
  );
});

/** Reasoning body. `memo`'d because the parent re-renders on every stream
 *  event (ChatStream re-renders per delta), and without it every such render
 *  would re-run the whole markdown pipeline over a multi-thousand-char chain
 *  whose text had not changed. Radix passes open/closed state through context,
 *  which bypasses memo, so the collapse animation still works.
 *
 *  Rendered through the same Streamdown pipeline as the answer — reasoning is
 *  markdown too (fenced code, inline code, lists, emphasis); it used to be
 *  `whitespace-pre-wrap` plain text, so those fences showed up literally. */
const ThinkingBody = memo(function ThinkingBody({
  text,
  animating,
  open,
}: {
  text: string;
  animating?: boolean;
  /** The parent's Collapsible open state — needed to gate the plain-div
   *  fallback that bypasses Radix's measured-height animation for huge
   *  traces. */
  open: boolean;
}) {
  const body = <ReasoningBody text={text} animating={animating} />;
  // A huge trace skips Radix's collapsible entirely: expanding it would pay a
  // synchronous getBoundingClientRect on the fresh subtree plus a 240ms
  // animated reflow — a plain open-gated div does neither.
  if (text.length > HUGE_TRACE_CHARS) {
    return open ? body : null;
  }
  return (
    <CollapsibleContent className="overflow-hidden data-[state=closed]:animate-collapsible-up data-[state=open]:animate-collapsible-down">
      {body}
    </CollapsibleContent>
  );
});

/** Past this length the reasoning body mounts without the collapse
 *  animation (and without Radix's content measurement). */
const HUGE_TRACE_CHARS = 16_000;

export function AssistantStatus({
  mode,
  thinkingText,
  startedAt,
  elapsedMs,
  doneLabel,
  children,
}: {
  mode:
    | "reply"
    | "thought"
    | "thinking"
    | "tools"
    | "tools_done"
    | "compacting"
    | null;
  thinkingText: string;
  /** Epoch ms when this phase ACTUALLY began — the thinking/tool item's own
   *  start stamp (or the turn's for the reply-wait row). Anchoring the clock
   *  here rather than on mount means a session switch / remount keeps the
   *  elapsed figure instead of restarting it at 0. Falls back to mount time
   *  when the caller has no stamp (e.g. a replayed item). */
  startedAt?: number;
  /** Settled span for a replayed row — `trace` stays null for a row that was
   *  never live this session, so the persisted `· 20s` renders statically. */
  elapsedMs?: number;
  /** Settled-mode label override — the tools row becomes "N 次工具调用"
   *  (count from the caller), exactly like "思考过程" carries the thinking
   *  phase's name. */
  doneLabel?: string;
  /** Collapsible body for non-thinking rows — the tools row mounts its chip
   *  list here so the status row IS the collapse toggle, one-to-one with
   *  the thinking row's expand-to-trace pattern. */
  children?: ReactNode;
}) {
  // Collapsed by default in EVERY mode — the header row alone carries the
  // state; reasoning body expands on explicit click. Previously the body
  // auto-opened while streaming, which pushed the answer down the viewport.
  const [open, setOpen] = useState(false);
  // Live time for the row's header. A row that MOUNTS settled (a thinking
  // block replayed from history, or one already finished when the turn
  // scrolled into view) starts with no clock at all — nothing was measured,
  // so nothing is shown.
  const [trace, setTrace] = useState<Trace | null>(() =>
    liveMode(mode) ? { banked: 0, since: startedAt ?? Date.now() } : null,
  );
  const [seenMode, setSeenMode] = useState(mode);
  if (mode !== seenMode) {
    setSeenMode(mode);
    // Read the clock OUT of the updater: React may invoke it twice
    // (StrictMode), and both passes must agree on the timestamp.
    const now = Date.now();
    setTrace((prev) => advanceClock(prev, liveMode(mode), now));
  }

  // Hooks must stay above the null-mode bail-out, so the mode-derived flag is
  // computed here rather than next to the label below.
  const live = liveMode(mode);
  if (!mode) return null;

  const label =
    mode === "reply"
      ? "正在回复"
      : mode === "tools"
        ? "正在调用工具"
        : mode === "compacting"
          ? "正在压缩上下文"
          : mode === "thinking"
            ? "思考中"
            : mode === "tools_done"
              ? (doneLabel ?? "工具调用")
              : "思考过程";
  const canToggle = Boolean(thinkingText) || children != null;
  /** Pinned while expanded — see the capsule note below. */
  const pinned = open && canToggle;

  return (
    <Collapsible open={open} onOpenChange={canToggle ? setOpen : undefined} className="w-full min-w-0 select-none my-1">
      {/* Pins the header to the top of the stream viewport while the block is
          expanded, so a long reasoning body can be read and collapsed without
          chasing the header back up. Gated on `open` rather than applied
          unconditionally because a sticky element costs a per-frame layout
          recalc on WebKitGTK (see `markdown-components.tsx`) and a collapsed
          row — just a chevron — has no body to scroll past it.

          Pinned, the row becomes a self-contained capsule instead of a
          full-width opaque bar: a mask only ever covers the strip it was
          sized for, so whatever it misses (row margins, a taller neighbour)
          reads as a half-covered gap — and a full-width sticky band also
          swallows clicks aimed at the content underneath. The capsule is
          opaque and `w-fit`, so it floats over the body, stays readable over
          text or a diff, and only its own area is interactive. */}
      <div className={cn(pinned && "sticky top-0 z-10 w-fit")}>
        <CollapsibleTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            aria-expanded={canToggle ? open : undefined}
            className={cn(
              "h-7 w-fit gap-1.5 text-xs font-medium text-neutral-500 hover:text-neutral-700",
              pinned
                ? "rounded-full border border-neutral-200 bg-white px-2.5 shadow-sm"
                : "px-1.5"
            )}
            disabled={!canToggle}
          >
            <span className="relative w-5 h-5 shrink-0 flex items-center justify-center">
              {/* Orbs mount only while the row is live — each one is a
               *  few hundred infinitely-animating spans, and opacity:0
               *  does NOT stop the animation engine, so a long session
               *  was paying N_rows × 3 × infinite keyframes forever. */}
              {live && (
                [
                  ["reply", "B3"],
                  ["thinking", "S1"],
                  ["tools", "S3"],
                  ["compacting", "S2"],
                ] as const
              ).map(([item, variant]) => (
                <span
                  key={item}
                  aria-hidden={mode !== item}
                  className={cn(
                    "absolute inset-0 flex items-center justify-center transition-opacity duration-300 ease-out",
                    mode === item ? "opacity-100" : "opacity-0 pointer-events-none"
                  )}
                >
                  <Orb label={label} variant={variant} size={14} className="text-neutral-800" />
                </span>
              ))}
              <span
                className={cn(
                  "absolute inset-0 flex items-center justify-center transition-opacity duration-300 ease-out",
                  live ? "opacity-0 pointer-events-none" : "opacity-100"
                )}
              >
                <ChevronDown
                  className={cn(
                    "h-3.5 w-3.5 text-neutral-500 transition-transform duration-300",
                    open ? "rotate-0" : "-rotate-90"
                  )}
                />
              </span>
            </span>
            <span className="relative">
              {(
                [
                  "正在回复",
                  "正在调用工具",
                  "正在压缩上下文",
                  "思考中",
                  "思考过程",
                  "工具调用",
                ] as const
              ).map(
                (item) => {
                  const active = label === item;
                  return (
                    <span
                      key={item}
                      aria-hidden={!active}
                      className={cn(
                        "whitespace-nowrap transition-opacity duration-300 ease-out",
                        active
                          ? "opacity-100"
                          : "pointer-events-none absolute top-0 left-0 opacity-0"
                      )}
                    >
                      {item}
                    </span>
                  );
                }
              )}
              {/* Dynamic settled label ("3 次工具调用") — the stack members
               *  cross-fade by opacity; width follows the in-flow active
               *  span, so a per-row count just renders as another member. */}
              {doneLabel != null && (
                <span
                  aria-hidden={label !== doneLabel}
                  className={cn(
                    "whitespace-nowrap transition-opacity duration-300 ease-out",
                    label === doneLabel
                      ? "opacity-100"
                      : "pointer-events-none absolute top-0 left-0 opacity-0"
                  )}
                >
                  {doneLabel}
                </span>
              )}
            </span>
            {/* `正在回复 · 20s` — how long this state has been running. */}
            {trace ? (
              <ElapsedLabel trace={trace} live={live} />
            ) : elapsedMs != null && elapsedMs >= 1000 ? (
              /* Replayed settled row — no clock ever ran this session, but
               * the persisted span still names how long it took. */
              <span className="whitespace-nowrap text-neutral-500 font-normal tabular-nums">
                · {formatElapsed(elapsedMs)}
              </span>
            ) : null}
          </Button>
        </CollapsibleTrigger>
      </div>
      {thinkingText ? (
        // `thinking` = the reasoning is still arriving → the streaming
        // highlighter, same as the answer text uses.
        <ThinkingBody
          text={thinkingText}
          animating={mode === "thinking"}
          open={open}
        />
      ) : (
        /* Non-thinking body (the tools row's chip list) — same Radix
         *  collapsible animation as the thinking trace: the content measures
         *  once and plays `animate-collapsible-*`, and unmounts when closed. */
        children != null ? (
          <CollapsibleContent>
            <div className="pt-0.5">{children}</div>
          </CollapsibleContent>
        ) : null
      )}
    </Collapsible>
  );
}
