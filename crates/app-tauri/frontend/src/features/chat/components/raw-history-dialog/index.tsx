// Raw history viewer — opens from the title-bar chart icon. A run inspector:
// a segmented view switch and search on top, a timeline strip showing when
// each round's input, model and tool time fell, then the event list grouped
// by round. Rows collapse by default; JSON is highlighted lazily.
import { memo, useEffect, useMemo, useRef, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Copy, Check, ChevronLeft, ChevronRight, RefreshCw, Search } from "@keyline-icons/react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { TooltipSimple } from "@/components/ui/tooltip";
import { highlightCodeToHtml } from "@/lib/syntax-highlight";
import { cn } from "@/lib/utils";
import { opensUserTurn } from "@/features/chat/hooks/view-from-history";
import type { ChatMessage } from "@/types";
import { toast } from "sonner";
import { readRawHistoryJson, useRawHistory } from "./use-raw-history";

type View = "time" | "round" | "tool";

/** Role → the coloured tag and the lane it draws on the timeline. */
const ROLE = {
  system: { label: "上下文", tag: "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300", bar: "bg-emerald-500" },
  user: { label: "用户", tag: "bg-sky-500/15 text-sky-700 dark:text-sky-300", bar: "bg-sky-500" },
  assistant: { label: "助手", tag: "bg-violet-500/15 text-violet-700 dark:text-violet-300", bar: "bg-violet-500" },
  tool: { label: "工具", tag: "bg-amber-500/15 text-amber-700 dark:text-amber-300", bar: "bg-amber-500" },
} as const;

type Role = keyof typeof ROLE;

const roleOf = (m: ChatMessage): Role =>
  m.role in ROLE ? (m.role as Role) : "system";

/** One conversation round: opens at a user message, holds the rest until the
 * next one. Messages before the first user turn form round 0. */
interface Round {
  index: number;
  items: { m: ChatMessage; at: number }[];
}

function groupRounds(messages: ChatMessage[], start: number, turnOffset: number): Round[] {
  const rounds: Round[] = [{ index: turnOffset, items: [] }];
  let turn = turnOffset;
  messages.forEach((m, at) => {
    // Match the store's turn index: hidden rows and steers do not start a turn.
    if (opensUserTurn(m)) {
      turn += 1;
      if (rounds[rounds.length - 1].items.length > 0) rounds.push({ index: turn, items: [] });
      else rounds[rounds.length - 1].index = turn;
    }
    rounds[rounds.length - 1].items.push({ m, at: start + at });
  });
  return rounds.filter((r) => r.items.length > 0);
}

function preview(m: ChatMessage): string {
  if (m.tool_calls?.length) {
    let text = "";
    for (const call of m.tool_calls) {
      text += `${call.function.name} ${call.function.arguments.slice(0, 280)}  `;
      if (text.length >= 280) break;
    }
    return text.slice(0, 280);
  }
  return (m.content ?? "").slice(0, 560).replace(/\s+/g, " ").trim().slice(0, 280);
}

function fmtDuration(ms?: number | null): string {
  if (!ms || ms <= 0) return "";
  return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(ms < 10000 ? 1 : 0)}s`;
}

/** Timeline: one labelled lane per kind of work, a shared time axis on top,
 * each block sized by its duration and placed by when it started. Hover a
 * block for what it was and how long it took. Position falls back to message
 * order when timestamps are missing (older snapshots). */
const LANES: { role: Role; name: string }[] = [
  { role: "system", name: "上下文" },
  { role: "user", name: "用户" },
  { role: "assistant", name: "模型" },
  { role: "tool", name: "工具" },
];

function Timeline({ messages, start }: { messages: ChatMessage[]; start: number }) {
  const stamped = messages.filter((m) => m.ts);
  const t0 = stamped.length ? Math.min(...stamped.map((m) => m.ts!)) : 0;
  const t1 = stamped.length ? Math.max(...stamped.map((m) => m.ts!)) : 0;
  const span = Math.max(t1 - t0, 1);
  const n = messages.length || 1;

  const blocks: Record<Role, { key: number; m: ChatMessage; left: number; width: number }[]> = {
    system: [], user: [], assistant: [], tool: [],
  };
  messages.forEach((m, at) => {
    const left = m.ts ? ((m.ts - t0) / span) * 100 : (at / n) * 100;
    const dur = m.duration_ms ?? 0;
    // A zero-length block still needs a visible mark.
    const width = Math.max(dur ? (dur / span) * 100 : 0, 0.8);
    blocks[roleOf(m)].push({ key: at, m, left: Math.min(left, 99.2), width });
  });

  const total = stamped.length ? fmtDuration(span) : "";
  const ticks = [0, 25, 50, 75, 100];
  return (
    <div className="flex-none border-b border-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] bg-neutral-50/60 px-4 pb-3 pt-2 dark:bg-white/[0.02]">
      {/* Time axis */}
      <div className="ml-16 flex justify-between pb-1.5 font-mono text-[10px] tabular-nums text-neutral-400">
        {ticks.map((t) => (
          <span key={t}>{total ? fmtDuration((span * t) / 100) || "0" : `${t}%`}</span>
        ))}
      </div>
      <div className="flex flex-col gap-1.5">
        {LANES.map(({ role, name }) => (
          <div key={role} className="flex items-center gap-2">
            <span className="w-14 flex-none text-right text-[11px] text-neutral-500">{name}</span>
            <div className="relative h-4 min-w-0 flex-1 rounded bg-neutral-200/50 dark:bg-white/[0.05]">
              {/* grid lines at the same ticks as the axis */}
              {ticks.slice(1, -1).map((t) => (
                <span key={t} className="absolute top-0 h-full w-px bg-neutral-300/40 dark:bg-white/[0.06]" style={{ left: `${t}%` }} />
              ))}
              {blocks[role].map((b) => (
                <TooltipSimple
                  key={b.key}
                  content={
                    <span className="flex flex-col gap-0.5">
                      <span className="font-medium">
                        #{start + b.key} · {ROLE[role].label}
                        {fmtDuration(b.m.duration_ms) && ` · ${fmtDuration(b.m.duration_ms)}`}
                      </span>
                      <span className="max-w-[260px] truncate opacity-80">{preview(b.m) || "（无内容）"}</span>
                    </span>
                  }
                >
                  <span
                    className={cn("absolute top-0.5 bottom-0.5 rounded-sm transition-opacity hover:opacity-100", ROLE[role].bar, "opacity-80")}
                    style={{ left: `${b.left}%`, width: `${b.width}%` }}
                  />
                </TooltipSimple>
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

const EventRow = memo(function EventRow({
  m,
  at,
  open,
  onToggle,
}: {
  m: ChatMessage;
  at: number;
  open: boolean;
  onToggle: () => void;
}) {
  const role = roleOf(m);
  const [copied, setCopied] = useState(false);
  // Stringify only when expanded — keeps the initial list cheap.
  const json = useMemo(() => (open ? JSON.stringify(m, null, 2) : ""), [open, m]);
  const html = useMemo(() => (open ? highlightCodeToHtml(json, "json") : ""), [open, json]);

  const copyOne = async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await navigator.clipboard.writeText(JSON.stringify(m, null, 2));
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {}
  };

  const dur = fmtDuration(m.duration_ms);
  return (
    <div className="border-b border-[color-mix(in_srgb,var(--husk-n200)_50%,transparent)] last:border-b-0">
      <button
        type="button"
        onClick={onToggle}
        className="group flex min-h-9 w-full items-center gap-3 px-4 py-1.5 text-left transition-colors hover:bg-neutral-100/70 dark:hover:bg-white/[0.04]"
      >
        <span className="w-8 flex-none text-right font-mono text-[10px] tabular-nums text-neutral-400">
          {at}
        </span>
        <Badge variant="secondary" className={cn("w-12 flex-none justify-center rounded-md px-0 text-[11px] font-medium", ROLE[role].tag)}>
          {ROLE[role].label}
        </Badge>
        <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-neutral-700">
          {preview(m)}
        </span>
        {dur && (
          <span className="flex-none font-mono text-[10.5px] tabular-nums text-neutral-400">
            {dur}
          </span>
        )}
        <Button
          variant="ghost"
          size="icon-sm"
          onClick={copyOne}
          aria-label="复制此条 JSON"
          title="复制此条 JSON"
          className="flex-none opacity-0 transition-opacity group-hover:opacity-100"
        >
          {copied ? <Check className="h-3 w-3 text-emerald-500" /> : <Copy className="h-3 w-3" />}
        </Button>
        <ChevronRight
          className={cn("h-3 w-3 flex-none text-neutral-400 transition-transform", open && "rotate-90")}
        />
      </button>
      {open && (
        <div className="mx-4 mb-2 max-h-[320px] overflow-auto rounded-lg border border-neutral-200 bg-neutral-50 dark:border-white/10 dark:bg-white/[0.03]">
          <pre
            className="whitespace-pre-wrap break-all p-3 font-mono text-[11px] leading-relaxed text-neutral-700"
            dangerouslySetInnerHTML={{ __html: html }}
          />
        </div>
      )}
    </div>
  );
});

export function RawHistoryDialog({
  open,
  onOpenChange,
  sessionId,
  workspaceRoot,
}: {
  open: boolean;
  onOpenChange: (v: boolean) => void;
  sessionId: number | null;
  workspaceRoot: string;
}) {
  const raw = useRawHistory(open, sessionId, workspaceRoot);
  const { page, loading, error } = raw;
  const messages = page?.history ?? null;
  const [copying, setCopying] = useState(false);
  const [copyProgress, setCopyProgress] = useState("");
  const copyAbort = useRef<AbortController | null>(null);

  // Row keys are absolute indices; drop expansion when the session changes.
  useEffect(() => setExpanded(new Set()), [sessionId, workspaceRoot]);
  useEffect(() => () => copyAbort.current?.abort(), []);
  const [view, setView] = useState<View>("round");
  const [query, setQuery] = useState("");
  const [expanded, setExpanded] = useState<Set<number>>(() => new Set());
  const [copiedAll, setCopiedAll] = useState(false);

  // The dialog stays mounted between openings, so its view, search and
  // expanded rows would carry over. Each opening starts from the default
  // view with no search — the loaded messages are kept, only the UI resets.
  const handleOpenChange = (next: boolean) => {
    if (!next) {
      setView("round");
      setQuery("");
      setExpanded(new Set());
    }
    onOpenChange(next);
  };

  const copyAll = async () => {
    if (sessionId === null || copying) return;
    const ctrl = new AbortController();
    copyAbort.current = ctrl;
    setCopying(true);
    try {
      const json = await readRawHistoryJson(sessionId, workspaceRoot, ctrl.signal, (read, total) =>
        setCopyProgress(`${read}/${total}`));
      await navigator.clipboard.writeText(json);
      setCopiedAll(true);
      setTimeout(() => setCopiedAll(false), 1500);
    } catch (e) {
      if (!ctrl.signal.aborted) toast.error(String(e));
    } finally {
      setCopying(false);
      setCopyProgress("");
    }
  };

  const rounds = useMemo(() => (messages ? groupRounds(messages, page?.history_start ?? 0, page?.turn_offset ?? 0) : []),
    [messages, page?.history_start, page?.turn_offset]);;

  // Search narrows every view to the matching messages.
  const q = query.trim().toLowerCase();
  const matches = (m: ChatMessage) =>
    !q || preview(m).toLowerCase().includes(q) || (m.content ?? "").toLowerCase().includes(q);

  const toggle = (at: number) =>
    setExpanded((s) => {
      const next = new Set(s);
      if (next.has(at)) next.delete(at);
      else next.add(at);
      return next;
    });

  const segments: { value: View; label: string }[] = [
    { value: "round", label: "轮次" },
    { value: "time", label: "时长" },
    { value: "tool", label: "调用" },
  ];

  // "调用" shows only the messages that did work: tool calls and their results.
  const shown = (items: { m: ChatMessage; at: number }[]) =>
    items.filter(({ m }) => matches(m) && (view !== "tool" || m.role === "tool" || m.tool_calls?.length));

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="flex h-[82vh] max-w-3xl flex-col gap-0 overflow-hidden p-0">
        <DialogHeader className="flex-none border-b border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)] px-4 pb-3 pt-3">
          <div className="flex items-center justify-between gap-3 pr-8">
            <div className="flex min-w-0 items-center gap-2">
              <DialogTitle className="flex-none text-[13px] font-semibold text-neutral-800">
                执行轨迹
              </DialogTitle>
              {page && (
                <Badge variant="secondary" className="h-6 flex-none px-2 font-mono text-[11px] font-normal tabular-nums leading-none">
                  {page.history_start + 1}–{page.history_end} / {page.history_total}
                </Badge>
              )}
              {page && (
                <span className="flex flex-none items-center gap-0.5 text-neutral-500">
                  <TooltipSimple content="更早">
                    <Button variant="ghost" size="icon-sm" className="h-6 w-6" onClick={raw.older}
                      disabled={loading || page.history_start === 0}>
                      <ChevronLeft className="h-3.5 w-3.5" />
                    </Button>
                  </TooltipSimple>
                  <TooltipSimple content="更新">
                    <Button variant="ghost" size="icon-sm" className="h-6 w-6" onClick={raw.newer}
                      disabled={loading || page.history_end >= page.history_total}>
                      <ChevronRight className="h-3.5 w-3.5" />
                    </Button>
                  </TooltipSimple>
                  <TooltipSimple content="刷新到最新">
                    <Button variant="ghost" size="icon-sm" className="h-6 w-6" onClick={raw.latest} disabled={loading}>
                      <RefreshCw className="h-3.5 w-3.5" />
                    </Button>
                  </TooltipSimple>
                </span>
              )}
            </div>
            <Button
              variant="ghost"
              size="sm"
              onClick={copyAll}
              disabled={sessionId === null || copying}
              className="h-7 flex-none gap-1.5 px-2 text-[11px] font-normal text-neutral-500"
            >
              {copying ? (
                <>
                  <RefreshCw className="h-3.5 w-3.5 animate-spin" />
                  复制中 {copyProgress}
                </>
              ) : copiedAll ? (
                <>
                  <Check className="h-3.5 w-3.5 text-emerald-500" />
                  已复制
                </>
              ) : (
                <>
                  <Copy className="h-3.5 w-3.5" />
                  复制全部 JSON
                </>
              )}
            </Button>
          </div>

          <div className="mt-3 flex items-center gap-3">
            <Tabs value={view} onValueChange={(v) => setView(v as View)}>
              <TabsList className="h-8">
                {segments.map((s) => (
                  <TabsTrigger key={s.value} value={s.value} className="px-3 text-[12px]">
                    {s.label}
                  </TabsTrigger>
                ))}
              </TabsList>
            </Tabs>
            <div className="relative ml-auto w-56">
              <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-neutral-400" />
              <Input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="搜索当前页"
                className="h-8 pl-8 text-[12px]"
              />
            </div>
          </div>
        </DialogHeader>

        {messages && messages.length > 0 && view === "time" && <Timeline messages={messages} start={page?.history_start ?? 0} />}

        <div className="min-h-0 flex-1 overflow-y-auto">
          {!open ? (
            // Closing: Radix keeps the tree mounted through the exit animation —
            // unmount the row list now so the zoom-out animates an empty shell.
            <div className="h-full" />
          ) : loading ? (
            <div className="flex h-full items-center justify-center text-[12px] text-neutral-500">正在读取…</div>
          ) : error ? (
            <div className="flex h-full flex-col items-center justify-center gap-2 text-[12px] text-neutral-500">
              <span>{error}</span>
              <Button variant="outline" size="sm" onClick={raw.retry}>重试</Button>
            </div>
          ) : !messages || messages.length === 0 ? (
            <div className="flex h-full items-center justify-center text-[12px] text-neutral-500">暂无记录</div>
          ) : view === "round" ? (
            rounds.map((r) => {
              const items = shown(r.items);
              if (items.length === 0) return null;
              return (
                <section key={r.index}>
                  <div className="sticky top-0 z-10 flex h-8 items-center gap-2 border-b border-neutral-100 bg-white px-4 py-1.5 dark:border-[var(--husk-border)] dark:bg-[var(--husk-hover)]">
                    <Badge variant="outline" className="h-5 px-1.5 font-mono text-[10px] font-normal leading-none">
                      第 {r.index + 1} 轮
                    </Badge>
                    <span className="text-[11px] tabular-nums leading-none text-neutral-400">{items.length} 条</span>
                  </div>
                  {items.map(({ m, at }) => (
                    <EventRow key={at} m={m} at={at} open={expanded.has(at)} onToggle={() => toggle(at)} />
                  ))}
                </section>
              );
            })
          ) : (
            shown(messages.map((m, i) => ({ m, at: (page?.history_start ?? 0) + i }))).map(({ m, at }) => (
              <EventRow key={at} m={m} at={at} open={expanded.has(at)} onToggle={() => toggle(at)} />
            ))
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
