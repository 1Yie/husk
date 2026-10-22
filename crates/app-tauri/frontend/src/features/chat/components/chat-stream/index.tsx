
import { memo, createContext, useContext, useEffect, useLayoutEffect, useRef, useState, useCallback, useMemo, startTransition, type ReactNode, type PointerEvent as ReactPointerEvent } from "react";
import { MemoStreamdown } from "@/features/chat/components/chat-stream/markdown-stream";
import type { SessionView, StreamItem } from "@/features/chat/hooks/stream-view";
import {
  AssistantStatus,
  formatElapsed,
} from "@/features/chat/components/assistant-status/index";
import { ChatSkeleton, OlderPageSkeleton } from "@/features/chat/components/chat-skeleton/index";
import { ToolChips, toolActivityPreview, type ToolChipRow } from "@/features/chat/components/tool-chips/index";
import {
  Check,
  Copy,
  ArrowDown,
  FileCode,
  Terminal,
  Sparkles,
  TextQuote,
  FileArrowRight,
  RotateCcw,
  GitReturn,
} from "@keyline-icons/react";
import { chatMarkdownComponents } from "@/features/chat/components/chat-stream/markdown-components";
import { TooltipSimple } from "@/components/ui/tooltip";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { ChatTurnRail, type RailMark } from "@/features/chat/components/chat-stream/chat-turn-rail";
import { opensTurn, railHistoryBoundary } from "./rail-history";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { toast } from "sonner";
import { requestQuote, selectionWithin } from "@/lib/selection-bus";
import { cn } from "@/lib/utils";
import { CircleCheck, CircleX, FileDiff, Package, PanelRightClose, PanelRightOpen } from "lucide-react";
import { Orb } from "@/features/chat/components/agent-orb";
import type { JobsSummary } from "@/lib/bg-jobs";
import { WAITING_STEER_HINT, summarizeChildren } from "@/lib/subagents";
import type { RunningChild } from "@/features/chat/hooks/running-children";
import { ChildSteerInput } from "@/features/chat/components/chat-stream/child-steer";
import { loadStamp } from "@/lib/load-probe";
import { timeGreeting } from "@/lib/greeting";
import { readAttachment } from "@/lib/agent-ipc/sessions";
import { retryTurn, undoLastTurn } from "@/lib/agent-ipc/commands";
import { CompactionCard } from "@/features/chat/components/chat-stream/compaction-card";
import { PlanCard } from "@/features/chat/components/chat-stream/plan-card";
import type { PlanPayload } from "@/types";

const BUILTIN_COMMANDS_DESC: Record<string, string> = {
  clear: "清空会话历史",
  compact: "压缩上下文以释放 token",
  undo: "回滚上一轮的文件修改",
  model: "切换模型 /model <provider>/<model>",
  diff: "查看本轮改动的文件",
  skills: "列出工作区可用 skills",
};

/** Image preview inside the `@`-chip tooltip — mounts on hover (Radix
 * portals content on open), resolves the staged path to a `data:` URL
 * via `read_attachment`, renders nothing while loading or on failure
 * (the tooltip still shows the filename + path rows below). */
function AttachedImagePreview({ path }: { path: string }) {
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    readAttachment(path)
      .then((a) => {
        if (alive && a.data_url) setSrc(a.data_url);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [path]);
  if (!src) return null;
  return (
    <img
      src={src}
      alt={path.split("/").pop()}
      className="max-h-40 max-w-[280px] w-auto rounded-md border border-[color-mix(in_srgb,var(--husk-n700)_50%,transparent)] object-contain bg-[color-mix(in_srgb,var(--husk-n950)_40%,transparent)]"
    />
  );
}

/** Interactive token chip in user chat bubble distinguishing @, /, and $ mentions. */
function UserTokenChip({ token }: { token: string }) {
  const clean = token.trim();

  if (clean.startsWith("@")) {
    const fullPath = clean.slice(1);
    const fileName = fullPath.split("/").filter(Boolean).pop() || fullPath;
    const isImage = /\.(png|jpe?g|gif|webp|bmp)$/i.test(fullPath);
    return (
      <TooltipSimple
        content={
          <div className="flex flex-col gap-1 text-left max-w-xs break-all py-0.5">
            {isImage && <AttachedImagePreview path={fullPath} />}
            <div className="flex items-center gap-1.5">
              <FileCode className="h-3.5 w-3.5 text-blue-400 dark:text-blue-600 shrink-0" />
              <span className="font-semibold text-neutral-100">{fileName}</span>
            </div>
            <div className="font-mono text-[11px] text-neutral-300 bg-[color-mix(in_srgb,var(--husk-n950)_70%,transparent)] rounded px-1.5 py-0.5 border border-[color-mix(in_srgb,var(--husk-n700)_60%,transparent)]">
              {fullPath}
            </div>
          </div>
        }
        side="top"
      >
        <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-[5px] text-[12px] font-mono font-medium transition-colors bg-blue-500/15 text-blue-700 dark:bg-blue-400/20 dark:text-blue-300 border border-blue-500/30 dark:border-blue-400/30 hover:bg-blue-500/25 cursor-default select-none mx-0.5 align-middle">
          <FileCode className="h-3.5 w-3.5 shrink-0 text-blue-600 dark:text-blue-400" />
          <span>@{fileName}</span>
        </span>
      </TooltipSimple>
    );
  }

  if (clean.startsWith("/")) {
    const cmdText = clean.slice(1);
    const [cmdName, ...argsList] = cmdText.split(" ");
    const args = argsList.join(" ").trim();
    const desc = BUILTIN_COMMANDS_DESC[cmdName];
    return (
      <TooltipSimple
        content={
          <div className="flex flex-col gap-1 text-left max-w-xs py-0.5">
            <div className="flex items-center gap-1.5">
              <Terminal className="h-3.5 w-3.5 text-violet-400 dark:text-violet-600 shrink-0" />
              <span className="font-semibold text-neutral-100">指令 /{cmdName}</span>
            </div>
            {desc && <span className="text-[11.5px] text-neutral-300">{desc}</span>}
            {args && (
              <div className="font-mono text-[11px] text-neutral-500 bg-[color-mix(in_srgb,var(--husk-n950)_70%,transparent)] rounded px-1.5 py-0.5 border border-[color-mix(in_srgb,var(--husk-n700)_60%,transparent)]">
                参数: {args}
              </div>
            )}
          </div>
        }
        side="top"
      >
        <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-[5px] text-[12px] font-mono font-medium transition-colors bg-violet-500/15 text-violet-700 dark:bg-violet-400/20 dark:text-violet-300 border border-violet-500/30 dark:border-violet-400/30 hover:bg-violet-500/25 cursor-default select-none mx-0.5 align-middle">
          <Terminal className="h-3.5 w-3.5 shrink-0 text-violet-600 dark:text-violet-400" />
          <span>{clean}</span>
        </span>
      </TooltipSimple>
    );
  }

  if (clean.startsWith("$")) {
    const skillText = clean.slice(1);
    const [skillName, ...argsList] = skillText.split(" ");
    const args = argsList.join(" ").trim();
    return (
      <TooltipSimple
        content={
          <div className="flex flex-col gap-1 text-left max-w-xs py-0.5">
            <div className="flex items-center gap-1.5">
              <Sparkles className="h-3.5 w-3.5 text-amber-400 dark:text-amber-600 shrink-0" />
              <span className="font-semibold text-neutral-100">技能 ${skillName}</span>
            </div>
            {args && (
              <div className="font-mono text-[11px] text-neutral-500 bg-[color-mix(in_srgb,var(--husk-n950)_70%,transparent)] rounded px-1.5 py-0.5 border border-[color-mix(in_srgb,var(--husk-n700)_60%,transparent)]">
                参数: {args}
              </div>
            )}
          </div>
        }
        side="top"
      >
        <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-[5px] text-[12px] font-mono font-medium transition-colors bg-amber-500/15 text-amber-700 dark:bg-amber-400/20 dark:text-amber-300 border border-amber-500/30 dark:border-amber-400/30 hover:bg-amber-500/25 cursor-default select-none mx-0.5 align-middle">
          <Sparkles className="h-3.5 w-3.5 shrink-0 text-amber-600 dark:text-amber-400" />
          <span>{clean}</span>
        </span>
      </TooltipSimple>
    );
  }

  return null;
}

const userMarkdownComponents = {
  ...chatMarkdownComponents,
  inlineCode: ({ children, ...props }: React.ComponentPropsWithoutRef<"code">) => {
    const text =
      typeof children === "string"
        ? children.trim()
        : Array.isArray(children)
          ? children.map((c) => (typeof c === "string" ? c : "")).join("").trim()
          : "";
    if (text.startsWith("@") || text.startsWith("/") || text.startsWith("$")) {
      return <UserTokenChip token={text} />;
    }
    return (
      <code
        className="bg-[color-mix(in_srgb,var(--husk-black)_5%,transparent)] text-neutral-800 px-1.5 py-[1px] rounded-[4px] text-[12px] font-mono border border-[color-mix(in_srgb,var(--husk-black)_7%,transparent)] font-normal mx-0.5 inline-block leading-snug align-baseline"
        {...props}
      >
        {children}
      </code>
    );
  },
};

const UserMemoStreamdown = memo(function UserMemoStreamdown({
  text,
}: {
  text: string;
}) {
  // Same chrome as the answer text; only the block styling differs.
  return <MemoStreamdown text={text} components={userMarkdownComponents} />;
});

/** The kernel expands `@path` mentions into `` `<workspace-file …>` `` +
 * fenced-content blocks, and `/skill`/`$skill` into a skill-prompt
 * preamble — that text is for the model. The user bubble shows only the
 * compact `@path` / `/name` chip. */
const WORKSPACE_FILE_BLOCK = /\s*`<workspace-file path="([^"]+)">`\s*```\n[\s\S]*?```\s*/g;
const ATTACHED_FILE_BLOCK = /\s*<attached-file path="([^"]+)">\s*```[\s\S]*?```\s*/g;
const ATTACHED_REF_BLOCK = /\s*\[attached (?:image|text|any): ([^\]]+?)(?: — [^\]]+)?\]\s*/g;
const ATTACHED_IMAGE_RE = /\s*<attached-image path="([^"]+)"(?:\s+name="([^"]*)")?\s*\/>\s*/g;
const SKILL_PROMPT_RE =
  /^The user invoked the `([/$][^\s`]+)` skill\. Follow its instructions exactly\.\n\n---\n[\s\S]*$/;

function collapsePromptArtifacts(md: string): string {
  const skill = SKILL_PROMPT_RE.exec(md);
  if (skill) {
    const args = /Skill arguments: ([\s\S]+)$/.exec(md)?.[1]?.trim();
    return `\`${skill[1]}${args ? ` ${args}` : ""}\``;
  }
  let res = md
    .replace(WORKSPACE_FILE_BLOCK, (_m, p) => `\`@${p}\` `)
    .replace(ATTACHED_FILE_BLOCK, (_m, p) => `\`@${p}\` `)
    .replace(ATTACHED_REF_BLOCK, (_m, p) => `\`@${p}\` `)
    .replace(ATTACHED_IMAGE_RE, (_m, p) => `\`@${p}\` `)
    .replace(/`(@[^`]+)` \((?:outside workspace|not found|binary file) — [^)]+\)/g, "`$1`")
    .replace(/\n{3,}/g, "\n\n")
    .trim();

  // Split by fenced code blocks (``` ... ```) so we never mutate code blocks
  const codeBlockParts = res.split(/(```[\s\S]*?```)/g);
  res = codeBlockParts
    .map((block, bIdx) => {
      if (bIdx % 2 === 1) return block;
      const inlineParts = block.split(/(`[^`]+`)/g);
      return inlineParts
        .map((part, iIdx) => {
          if (iIdx % 2 === 1) return part;
          // In plain text, wrap bare @path, /cmd, $skill in backticks so markdown parses them as inlineCode
          return part.replace(
            /(^|\s)(@(?!@)[^\s`]+|\$(?!\$)[a-zA-Z0-9_.-]+|\/(?!\/)[a-zA-Z0-9_.-]+)(?=\s|[.,;:!?，。！？]|$)/g,
            "$1`$2`",
          );
        })
        .join("");
    })
    .join("");

  return res;
}

/** Empty state for a fresh session — a large centered greeting plus a
 * hint pointing at the composer, replacing the old one-line caption. */
function EmptyGreeting() {
  const { label, tail, Icon, tone } = timeGreeting();
  return (
    <div className="my-auto flex flex-col items-center gap-3 select-none pb-16">
      <span className="flex items-center gap-3 text-[26px] leading-snug font-medium tracking-tight text-neutral-800">
        <Icon size={24} className={tone} />
        {label}
        {tail}
      </span>
      <span className="text-[13px] text-neutral-500">
        在下方输入框发送消息，开始新对话
      </span>
    </div>
  );
}

function getUserPreview(userText?: string): string {
  if (!userText) return "用户提问";
  return collapsePromptArtifacts(userText).replace(/[`]/g, "").replace(/\s+/g, " ").trim();
}

function getAssistantPreview(steps: AssistantStep[]): string {
  for (const step of steps) {
    if (step.type === "text" && step.text.trim()) {
      // Slice before collapsing — the whitespace regex is O(text) and a
      // thinking trace can run to hundreds of KB; 2k chars still collapse
      // well past the 160-char clip.
      return step.text.slice(0, 2048).trim().replace(/\s+/g, " ");
    }
  }
  for (const step of steps) {
    if (step.type === "thinking" && step.text.trim()) {
      return `思考: ${step.text.slice(0, 2048).trim().replace(/\s+/g, " ")}`;
    }
  }
  for (const step of steps) {
    if (step.type === "tools" && step.rows.length > 0) {
      return `工具: ${step.rows.map((r) => r.label).join(", ")}`;
    }
  }
  for (const step of steps) {
    if (step.type === "compaction")
      return step.pending ? "正在压缩上下文" : "上下文已压缩";
  }
  for (const step of steps) {
    if (step.type === "plan") {
      return step.plan.summary.trim() || "计划已提交";
    }
  }
  for (const step of steps) {
    if (step.type === "system" && step.text.trim()) {
      return step.text.trim();
    }
  }
  return "助手已回复";
}

interface Props {
  view: SessionView;
  /** Extra bottom room so the last message can scroll above the floating
   * composer — measured from the real composer height upstream. */
  bottomPad?: number;
  /** Real composer height measured in ChatPage, used to position the
   * floating scroll-to-bottom button exactly above the composer card. */
  composerH?: number;
  /** True while a session/workspace switch is fetching + rebuilding the
   * view — renders the skeleton instead of the (stale or empty) stream. */
  loading?: boolean;
  /** Identity of the session on screen (`root:id`) — resets the
   * incremental render window so a long session mounts its tail first. */
  sessionKey?: string;
  /** Older-history pages exist above the loaded slice. */
  hasMore?: boolean;
  /** Right dock (改动 + 产物) — its toggle shares this centered floating row
   *  with 回到底部, so both sit on the same axis above the composer. */
  dockOpen?: boolean;
  /** Files in the dock's active view. */
  dockCount?: number;
  /** Which view the dock is showing — the pill names it, so the sidebar's
   *  state is readable without looking right. */
  dockTab?: "changes" | "artifacts" | null;
  /** Line totals of the 改动 view — the pill shows the diff's own numbers. */
  dockAdds?: number;
  dockDels?: number;
  onToggleDock?: () => void;
  /** The session's background jobs in one line — its own pill beside the
   *  dock's, since jobs are not files. `null` = no jobs, no pill. */
  jobs?: JobsSummary | null;
  /** The dock is open on the 后台 view. */
  jobsOpen?: boolean;
  onToggleJobs?: () => void;
  /** Delegated subagents still running — their own capsule beside the jobs
   *  pill, listing what each child was handed. `[]` = no pill. */
  subagents?: RunningChild[];
  /** Fetch + prepend the next older page (scroll-top trigger). */
  /** Fetch + prepend the next older page. Resolves `true` when a
   * page actually landed — `false` (empty/end) clears the pending
   * scroll-restore so a stale geometry can't be consumed later. */
  onLoadOlder?: () => Promise<boolean>;
}

type AssistantStep =
  | {
      type: "thinking";
      text: string;
      done: boolean;
      startedAt?: number;
      /** Persisted block span (ms) — the settled row's `· 20s` on replay. */
      elapsedMs?: number;
    }
  | { type: "text"; text: string; streaming: boolean }
  | {
      type: "tools";
      rows: ToolChipRow[];
      startedAt?: number;
      /** Latest tool-result stamp (epoch ms) — the fallback phase end when
       *  nothing followed the step (turn ended right on the tools). */
      endedAt?: number;
      /** First NON-tool item's arrival after this step — the true phase end.
       *  The tools phase includes the model-wait between/after calls, so
       *  `phaseEnd ?? endedAt` is the settled span; endedAt alone measures
       *  only up to the last dispatch's finish. */
      phaseEnd?: number;
    }
  | {
      type: "compaction";
      before: number;
      after: number;
      removed: number;
      note: string;
      /** Pass still running — the card renders as a progress placeholder. */
      pending?: boolean;
      /** `true` = the user ran `/compact`; renders as its own block. */
      manual?: boolean;
    }
  | { type: "system"; text: string }
  | { type: "plan"; plan: PlanPayload };

/** Mounted-step window: only steps intersecting (plus overscan) this scroll
 *  root get their real TurnStep. Placeholders carry the rail target ids. */
interface VirtualStepApi {
  observe: (el: Element, onChange: (visible: boolean) => void) => () => void;
  /** Synchronous "is this inside the window the reader will see?" — read in
   *  a step's first layout pass so the steps on screen mount BEFORE the
   *  first paint. The observer's first report only arrives after a frame,
   *  which painted every visible text block as a blank placeholder (the
   *  white flash when the skeleton lifted). */
  isNear: (el: Element) => boolean;
}

const VirtualStepContext = createContext<VirtualStepApi | null>(null);

interface Turn {
  id: string;
  /** Global history index of the turn's first item — lets a placeholder
   * click-seek target the right turn after its page loads. */
  hi?: number;
  userText?: string;
  /** The bubble is a mid-turn steer: drawn like a prompt, but not a turn of
   * its own — it never advances a global turn ordinal. */
  steer?: boolean;
  /** Epoch ms of the user message — footer timestamp on the bubble. */
  ts?: number;
  /** Epoch ms of the last assistant item — the turn-end stamp under the
   * assistant block. Falls back to `ts` when a turn produced no text. */
  assistantTs?: number;
  steps: AssistantStep[];
}

/** `—— HH:mm ——` / `—— MM-DD HH:mm ——` / `—— YYYY-MM-DD HH:mm ——` —
 * progressively more context the older the turn is. */
function formatTurnTime(ts: number): string {
  const d = new Date(ts);
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  const hm = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  if (sameDay) return hm;
  const md = `${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  if (d.getFullYear() === now.getFullYear()) return `${md} ${hm}`;
  return `${d.getFullYear()}-${md} ${hm}`;
}

/** Full timestamp for the divider tooltip — date, time with seconds,
 * GMT offset and IANA zone: `2025-09-20 17:37:42 GMT+8 (Asia/Shanghai)`. */
function formatFullTime(ts: number): string {
  const d = new Date(ts);
  const pad = (n: number) => String(n).padStart(2, "0");
  const date = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  const time = `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
  const offsetMin = -d.getTimezoneOffset();
  const sign = offsetMin >= 0 ? "+" : "-";
  const hours = Math.abs(offsetMin) / 60;
  const gmt = `GMT${sign}${Number.isInteger(hours) ? hours : hours.toFixed(1)}`;
  const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  return `${date} ${time} ${gmt} (${zone})`;
}

/** Tiny ghost icon-button for the per-message footers. */
function FooterBtn({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <TooltipSimple content={label} side="bottom">
      <button
        type="button"
        aria-label={label}
        onClick={onClick}
        className="h-5 w-5 flex items-center justify-center rounded-md text-neutral-500 hover:text-neutral-700 hover:bg-neutral-100 transition-colors cursor-pointer"
      >
        {children}
      </button>
    </TooltipSimple>
  );
}

/** Per-side message footer — tiny actions, timestamp trailing last. */
function TurnFooter({
  ts,
  copyText,
  onRetry,
  onUndo,
  align,
  durationMs,
}: {
  ts: number;
  copyText?: string;
  onRetry?: () => void;
  onUndo?: () => void;
  align: "start" | "end";
  /** Turn wall time (end stamp − user stamp) — rendered as `耗时 Xs`. */
  durationMs?: number;
}) {
  const [copied, setCopied] = useState(false);
  const [retryOpen, setRetryOpen] = useState(false);
  const [undoOpen, setUndoOpen] = useState(false);
  const copy = async () => {
    if (!copyText) return;
    try {
      await navigator.clipboard.writeText(copyText);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      toast.error("复制失败");
    }
  };
  return (
    <div
      className={cn(
        "flex items-center gap-1.5 text-[11px] text-neutral-500 select-none",
        align === "end" ? "justify-end" : "justify-start"
      )}
    >
      {copyText != null && copyText.trim() !== "" && (
        <FooterBtn label="复制" onClick={() => void copy()}>
          {copied ? (
            <Check className="h-3.5 w-3.5" />
          ) : (
            <Copy className="h-3.5 w-3.5" />
          )}
        </FooterBtn>
      )}
      {onRetry && (
        <>
          <FooterBtn label="重试" onClick={() => setRetryOpen(true)}>
            <RotateCcw className="h-3.5 w-3.5" />
          </FooterBtn>
          <Dialog open={retryOpen} onOpenChange={setRetryOpen}>
            <DialogContent className="max-w-sm">
              <DialogHeader>
                <DialogTitle className="text-base">重试回复</DialogTitle>
                <DialogDescription>
                  将移除当前回复，并从你的消息重新生成。该操作不可撤销。
                </DialogDescription>
              </DialogHeader>
              <DialogFooter className="gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => setRetryOpen(false)}
                >
                  取消
                </Button>
                <Button
                  size="sm"
                  onClick={() => {
                    setRetryOpen(false);
                    onRetry();
                  }}
                >
                  重试
                </Button>
              </DialogFooter>
            </DialogContent>
          </Dialog>
        </>
      )}
      {onUndo && (
        <>
          <FooterBtn label="撤销" onClick={() => setUndoOpen(true)}>
            <GitReturn className="h-3.5 w-3.5" />
          </FooterBtn>
          <Dialog open={undoOpen} onOpenChange={setUndoOpen}>
            <DialogContent className="max-w-sm">
              <DialogHeader>
                <DialogTitle className="text-base">撤销本轮修改</DialogTitle>
                <DialogDescription>
                  回滚上一轮 agent 写入的全部文件。已被外部修改过的文件会跳过。
                </DialogDescription>
              </DialogHeader>
              <DialogFooter className="gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => setUndoOpen(false)}
                >
                  取消
                </Button>
                <Button
                  size="sm"
                  onClick={() => {
                    setUndoOpen(false);
                    onUndo();
                  }}
                >
                  撤销
                </Button>
              </DialogFooter>
            </DialogContent>
          </Dialog>
        </>
      )}
      <span
        className="cursor-default px-0.5 tabular-nums"
        data-tooltip={formatFullTime(ts)}
      >
        {formatTurnTime(ts)}
      </span>
      {durationMs != null && durationMs >= 1000 && (
        <span className="cursor-default tabular-nums text-neutral-400">
          耗时 {formatElapsed(durationMs)}
        </span>
      )}
    </div>
  );
}

/** The copyable text of a turn's assistant side — text steps joined,
 * thinking/tool rows excluded (copy = the answer, not the trace). */
function assistantCopyText(turn: Turn): string {
  return turn.steps
    .flatMap((s) => (s.type === "text" ? [s.text] : []))
    .join("\n\n")
    .trim();
}

/** A parsed turn plus the exact item slice it was folded from — the
 * span lets the next render reuse the turn untouched when every item
 * reference still matches (stream events only ever rewrite the tail). */
interface TurnSpan {
  turn: Turn;
  span: StreamItem[];
}

/** Fold items[start..] into turn spans, appended to `out`. Identical
 * output to the old single-pass `parseTurns`, just callable on a tail
 * slice so untouched leading turns can be reused by reference. */
function foldTurnSpans(items: StreamItem[], start: number, end: number, out: TurnSpan[]) {
  let currentTurn: Turn | null = null;
  let turnStart = start;

  // The current turn's child cards by id — a nested item finds its card by id
  // whichever step it landed in, so a mid-turn tools step can't strand it.
  let childRows = new Map<number, ToolChipRow>();

  const flush = (end: number) => {
    if (currentTurn) {
      out.push({ turn: currentTurn, span: items.slice(turnStart, end) });
      currentTurn = null;
      childRows = new Map();
    }
  };

  // Stable id: `hi` (global history index) survives prepended pages;
  // `ts` (message timestamp) does too; index is the live-only fallback.
  const turnId = (item: StreamItem, idx: number) =>
    item.hi != null
      ? `t${item.hi}`
      : item.kind === "user" && item.ts != null
        ? `ts${item.ts}`
        : `i${idx}`;

  const ensureTurn = (item: StreamItem, idx: number) => {
    if (!currentTurn) {
      currentTurn = { id: turnId(item, idx), hi: item.hi, steps: [] };
      turnStart = idx;
    }
    return currentTurn;
  };

  for (let idx = start; idx < end; idx++) {
    const item = items[idx];

    if (item.kind === "user") {
      flush(idx);
      currentTurn = {
        id: turnId(item, idx),
        hi: item.hi,
        userText: item.text,
        steer: item.steer,
        ts: item.ts,
        steps: [],
      };
      turnStart = idx;
      continue;
    }

    if (item.kind === "compaction") {
      // Whether a compaction pass splits the stream is a POSITION
      // question, not a trigger question. `manual` only says who asked
      // for it (auto threshold vs `/compact`) — a `/compact` issued
      // mid-turn still lands INSIDE the running turn (the engine's
      // `ControlOp::Compact` doesn't wait for `Finished`), so splitting
      // on `manual` cut one logical turn in two, which is the visible
      // "split into two pieces" bug. `currentTurn` is the truth: null =
      // between turns (own block), non-null = fold into the open turn's
      // steps regardless of `manual`.
      const step = {
        type: "compaction" as const,
        before: item.before,
        after: item.after,
        removed: item.removed,
        note: item.note,
        manual: item.manual,
        pending: item.pending,
      };
      if (!currentTurn) {
        flush(idx);
        out.push({
          turn: {
            id: turnId(item, idx),
            hi: item.hi,
            steps: [step],
          },
          span: items.slice(idx, idx + 1),
        });
        continue;
      }
      const t = ensureTurn(item, idx);
      t.steps.push(step);
      continue;
    }

    // A between-turns notice is its own block below the previous turn.
    if (item.kind === "system" && item.standalone) {
      flush(idx);
      out.push({
        turn: {
          id: turnId(item, idx),
          hi: item.hi,
          steps: [{ type: "system", text: item.text }],
        },
        span: items.slice(idx, idx + 1),
      });
      continue;
    }

    const t = ensureTurn(item, idx);
    const lastStep = t.steps[t.steps.length - 1];

    // A non-tool item landing after a tools step marks the phase's END —
    // the span covers the whole phase (call → model-wait → next call), not
    // just the last dispatch. FIRST follower wins: the assistant commit `ts`
    // arriving later must not stretch it into the answer's write time.
    if (
      item.kind !== "tool" &&
      item.kind !== "approval" &&
      lastStep?.type === "tools" &&
      lastStep.phaseEnd == null
    ) {
      const stamp =
        ("startedAt" in item ? item.startedAt : undefined) ??
        ("ts" in item ? item.ts : undefined);
      if (stamp != null) lastStep.phaseEnd = stamp;
    }

    if (item.kind === "thinking") {
      t.steps.push({
        type: "thinking",
        text: item.text,
        done: item.done,
        startedAt: item.startedAt,
          // Settled span — `ts` is stamped when the block closes (live:
          // `closeOpenThinking`; replay: persisted `m.ts`), so the figure
          // survives a session switch that reuses the cached view.
          elapsedMs:
            item.done && item.ts != null && item.startedAt != null
              ? item.ts - item.startedAt
              : undefined,
      });
      continue;
    }

    if (item.kind === "assistant") {
      if (item.ts != null) t.assistantTs = item.ts;
      if (lastStep?.type === "text") {
        lastStep.text = item.text;
        lastStep.streaming = item.streaming;
      } else {
        t.steps.push({
          type: "text",
          text: item.text,
          streaming: item.streaming,
        });
      }
      continue;
    }

    if (item.kind === "system") {
      t.steps.push({
        type: "system",
        text: item.text,
      });
      continue;
    }

    if (item.kind === "plan") {
      // The submitted plan is turn content — the card that ends it.
      t.steps.push({ type: "plan", plan: item.plan });
      continue;
    }

    if (item.kind === "tool" || item.kind === "approval") {
      // Batch children and a subagent's own calls — attach to their parent row
      // instead of counting as a top-level call.
      if (item.kind === "tool" && item.parent) {
        {
          // A delegated child's id names its card exactly; the label is only
          // the fallback (two children can share one).
          const byId = item.parent !== "delegate" && item.childId != null
            ? childRows.get(item.childId)
            : undefined;
          const findParent = (rows: ToolChipRow[]): ToolChipRow | null => {
            for (let j = rows.length - 1; j >= 0; j--) {
              if (rows[j].label === item.parent) return rows[j];
              const nested = rows[j].children ? findParent(rows[j].children!) : null;
              if (nested) return nested;
            }
            return null;
          };
          // Parent may be a top-level row (batch) or a nested agent card (a
          // subagent's own tool calls) — search the WHOLE turn, newest first,
          // not just its last step: a child's card can sit steps earlier.
          const scanTurn = (): ToolChipRow | null => {
            for (let s = t.steps.length - 1; s >= 0; s--) {
              const step = t.steps[s];
              if (step.type !== "tools") continue;
              const hit = findParent(step.rows);
              if (hit) return hit;
            }
            return null;
          };
          const parentRow = byId ?? scanTurn();
          if (parentRow) {
              const child: ToolChipRow = {
                id: item.hi != null ? `tool-${item.hi}` : `tool-${idx}`,
                // A subagent is a second agent, not a tool call: give it its
                // own card so it does not read as one more line in the list.
                kind: item.parent === "delegate" ? "agent" : undefined,
                // A delegated child's own card: carry its id and its ordered body so
                // a running card can show text and tool runs interleaved.
                childId: item.parent === "delegate" ? item.childId : undefined,
                segments: item.parent === "delegate" ? item.segments : undefined,
                label: item.name,
                chip:
                  item.parent === "delegate"
                    ? (item.args ?? "")
                    : item.args && item.args !== item.name
                      ? item.args
                      : "",
                status:
                  item.uiType === "rejected"
                    ? "rejected"
                    : item.content === undefined
                      ? "running"
                      : item.ok === false
                        ? "aborted"
                        : "done",
                // Finished → the report; still running → whatever answer text
                // it streamed (its reasoning is its own 思考 block).
                detail: item.content ? [item.content] : item.live ? [item.live] : undefined,
                reasoning: item.liveReasoning,
                waiting: item.waiting,
                uiType: item.uiType,
              };
              if (child.childId != null) childRows.set(child.childId, child);
              parentRow.children = [...(parentRow.children ?? []), child];
              continue;
          }
          // No parent found — fall through and draw it as a top-level row
          // rather than dropping the event.
        }
      }
      const row: ToolChipRow =
        item.kind === "tool"
          ? {
              id: item.hi != null ? `tool-${item.hi}` : `tool-${idx}`,
              // A delegated subagent's own card: a second agent, so it gets the
              // agent card and carries the id a `SteerChild` names.
              kind: item.parent === "delegate" ? "agent" : undefined,
              childId: item.childId,
              segments: item.segments,
              label: item.name,
              chip: item.args && item.args !== item.name ? item.args : "",
              status:
                item.uiType === "rejected"
                  ? "rejected"
                  : item.content === undefined
                    ? "running"
                    : item.ok === false
                      ? "aborted"
                      : "done",
              detail: item.content
                ? [item.content]
                : item.approval?.diff
                  ? [item.approval.diff]
                  : undefined,
              reasoning: item.liveReasoning,
              waiting: item.waiting,
              uiType: item.uiType,
              approval: item.approval
                ? {
                    requestId: item.approval.requestId,
                    diff: item.approval.diff || "",
                    resolved: item.approval.resolved,
                    approved: item.approval.approved,
                  }
                : undefined,
            }
          : {
              id: `approval-${item.requestId}`,
              label: item.toolName,
              chip: "请求批准",
              status: item.resolved ? (item.approved ? "done" : "aborted") : "running",
              detail: item.diff ? [item.diff] : undefined,
              uiType: item.diff ? "diff" : undefined,
              approval: {
                requestId: item.requestId,
                diff: item.diff,
                resolved: item.resolved,
                approved: item.approved,
              },
            };

      // A child card is findable by id from anywhere later in this turn.
      if (row.kind === "agent" && row.childId != null) childRows.set(row.childId, row);
      if (lastStep?.type === "tools") {
        lastStep.rows.push(row);
        // The step's clock anchors on its FIRST tool — later calls in the
        // same tools phase extend the run, they don't restart it.
        if (lastStep.startedAt == null && item.kind === "tool") {
          lastStep.startedAt = item.startedAt;
        }
        // The step's END anchors on its latest tool result — persisted rows
        // carry `ts`, live ones stamp it on `ToolCallFinished`.
        if (item.kind === "tool" && item.ts != null) {
          lastStep.endedAt = Math.max(lastStep.endedAt ?? 0, item.ts);
        }
      } else {
        t.steps.push({
          type: "tools",
          rows: [row],
          startedAt: item.kind === "tool" ? item.startedAt : undefined,
          endedAt: item.kind === "tool" ? item.ts : undefined,
        });
      }
    }
  }

  flush(end);
}

/** Structural equality over fold output (plain objects, arrays, primitives).
 * Strings shared with the items compare by reference, so this is cheap. */
function sameShape(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (Array.isArray(a)) {
    if (!Array.isArray(b) || a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (!sameShape(a[i], b[i])) return false;
    return true;
  }
  if (Array.isArray(b)) return false;
  const ka = Object.keys(a);
  if (ka.length !== Object.keys(b).length) return false;
  for (const k of ka) {
    if (!sameShape((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k])) return false;
  }
  return true;
}

/** A refolded tail turn is built from fresh objects on every flush. Hand back
 * the previous fold's step objects (and the previous turn itself) wherever the
 * content is unchanged, so a long running turn re-renders only the step that
 * is actually growing instead of every step it has accumulated. */
function reuseTurn(prev: Turn | undefined, next: Turn): Turn {
  if (!prev || prev.id !== next.id) return next;
  let allSame = prev.steps.length === next.steps.length;
  for (let i = 0; i < next.steps.length; i++) {
    const old = prev.steps[i];
    if (old && sameShape(old, next.steps[i])) next.steps[i] = old;
    else allSame = false;
  }
  if (
    allSame &&
    prev.userText === next.userText &&
    prev.ts === next.ts &&
    prev.assistantTs === next.assistantTs &&
    prev.hi === next.hi
  ) {
    return prev;
  }
  return next;
}

/** Turns mount all at once, behind the loading veil; `content-visibility: auto` then
 *  skips offscreen layout and paint, so scrolling costs nothing per turn. */

/** Per-turn preview strings — WeakMap'd on the cache-stable Turn so the
 * regex-heavy collapse only runs once per turn instead of once per flush
 * (every flush used to re-run it over EVERY user message). */
const userPreviewCache = new WeakMap<Turn, string>();

function userPreviewOf(turn: Turn): string {
  let s = userPreviewCache.get(turn);
  if (s === undefined) {
    s = getUserPreview(turn.userText);
    userPreviewCache.set(turn, s);
  }
  return s;
}

/** Rail label for the kind of block a step draws. */
const STEP_LABEL: Record<AssistantStep["type"], string> = {
  thinking: "思考",
  text: "输出",
  tools: "工具",
  compaction: "压缩",
  system: "提示",
  plan: "计划",
};

/** Rail preview + weight per step — WeakMap'd on the step object, which the
 * tail reconcile keeps stable, so a flush only recomputes the growing step. */
/** Rail dashes per answer — longer answers fold step runs into one dash. */
const MAX_STEP_MARKS = 6;

const stepMarkCache = new WeakMap<AssistantStep, { preview: string; weight: number }>();

function stepMarkInfo(step: AssistantStep): { preview: string; weight: number } {
  let info = stepMarkCache.get(step);
  if (!info) {
    info = { preview: stepPreview(step), weight: stepWeight(step) };
    stepMarkCache.set(step, info);
  }
  return info;
}

/** One-line rail preview; clipped before the whitespace collapse. */
function stepPreview(step: AssistantStep): string {
  const raw = getAssistantPreview([step]);
  const cut = clipPreview(raw);
  // The mark's own label names the block, so the "思考: " prefix is dropped.
  return cut.replace(/^(思考|工具):\s*/, "").replace(/\s+/g, " ").trim();
}

/** Rail tooltips get one line: 160 characters, then an ellipsis. */
function clipPreview(text: string, max = 160): string {
  return text.length > max ? `${text.slice(0, max).trimEnd()}…` : text;
}

/** Block weight in characters. A tool step counts its cards: the card body
 *  (`detail` — streamed text, report, diff, log) is what makes a turn long to
 *  scroll, not its prose. */
function stepWeight(step: AssistantStep): number {
  // The compaction card is a fixed-height header; the expandable summary
  // counts only once its own toggle opens (not part of the collapse weight).
  if (step.type === "compaction") return 160;
  // The plan card is a fixed-height block too — its content is reviewed
  // on the card, not weighed for collapse.
  if (step.type === "plan") return 200;
  if (step.type !== "tools") return step.text.length;
  let n = 0;
  for (const row of step.rows) {
    n += row.label.length + 40;
    if (row.detail) {
      for (const line of row.detail) n += line.length + 1;
    }
  }
  return n;
}

/** One text block can carry a paste-sized payload and streamdown renders every
 *  block, so a single message can mount tens of thousands of nodes. Cap the rendered
 *  prefix behind an explicit expand — never while streaming. */
const MAX_TEXT_CHARS = 15000;

function sliceAtBoundary(text: string, max: number): string {
  let end = text.lastIndexOf("\n", max);
  if (end < max * 0.5) end = max; // no good boundary nearby — hard slice
  let head = text.slice(0, end);
  // Close an unterminated code fence so the tail's fence state doesn't
  // leak into the preview render.
  const fences = (head.match(/```/g) || []).length;
  if (fences % 2 === 1) head += "\n```";
  return head;
}

function CappedAssistantText({ text, animating }: { text: string; animating?: boolean }) {
  const [expanded, setExpanded] = useState(false);
  const capped = !animating && !expanded && text.length > MAX_TEXT_CHARS;
  const shown = capped ? sliceAtBoundary(text, MAX_TEXT_CHARS) : text;
  return (
    <>
      {/* The answer is the one surface that opts into the per-character fade:
          it is what the reader is watching arrive. */}
      <MemoStreamdown text={shown} animating={animating} streamFx="animate" />
      {capped && (
        <button
          type="button"
          onClick={() => setExpanded(true)}
          className="mt-1 text-[12px] text-neutral-500 hover:text-neutral-600 select-none"
        >
          … 还有 {text.length - shown.length} 字符，点击展开全部
        </button>
      )}
    </>
  );
}

/** Anything Streamdown renders differently from plain text: inline markup,
 *  headings/quotes/lists (bulleted and numbered), tables, HTML, links and
 *  autolinks, `$…$` inline math (the renderer enables single-dollar), and
 *  backslash escapes. `\` last in the class — JS regex needs it doubled. */
const PLAIN_MARKUP = /[`*_#[\]|<>~$\\]|https?:\/\/|^\s*(?:[-+>]|\d+[.)])\s/m;

/** One assistant step. `memo` + the tail reconcile in `turns` keep a settled
 * step's object stable, so a delta in a long running turn re-renders only the
 * step that is growing — not every tool row and trace the turn has collected.
 * `streaming` only matters to the last step, so earlier steps ignore it. */
const TurnStep = memo(
  function TurnStep({
    step,
    turnId,
    stepIdx,
    isLastStep,
    streaming,
    allowPlain = true,
  }: {
    step: AssistantStep;
    turnId: string;
    stepIdx: number;
    isLastStep: boolean;
    streaming: boolean;
    /** The plain-text fast path may be used — false for a step that mounted
     *  as the live tail (see the text branch). */
    allowPlain?: boolean;
  }) {
    if (step.type === "thinking") {
      const thinkingText = step.text.trim();
      if (!step.done && !thinkingText) return null;
      return (
        <div
          className="w-full"
        >
          <AssistantStatus
            mode={step.done ? "thought" : "thinking"}
            thinkingText={thinkingText}
            startedAt={step.startedAt}
            elapsedMs={step.elapsedMs}
          />
        </div>
      );
    }

    if (step.type === "text") {
      const isAnimating = streaming && isLastStep;
      // A tool round usually opens with a one-line preamble ("Let me check
      // …"). Those short, markup-free non-final blocks don't need a full
      // Streamdown pipeline; mounting dozens of them while scrolling was a
      // large part of the jank. Final answers always use Streamdown. The
      // choice is fixed when the step mounts (`allowPlain`): a block mounted
      // while it was the live tail must not swap renderers — and reflow —
      // the moment a tool call makes it "non-last".
      const plain =
        allowPlain &&
        !isLastStep &&
        step.text.length <= 280 &&
        !PLAIN_MARKUP.test(step.text);

      return (
        <div
          className="w-full min-w-0 text-[14px] text-neutral-900 [&_p]:max-w-none"
        >
          {plain ? (
            <div className="whitespace-pre-wrap leading-relaxed break-words">{step.text}</div>
          ) : (
            <CappedAssistantText text={step.text} animating={isAnimating} />
          )}
        </div>
      );
    }

    if (step.type === "tools") {
      // The tools phase is CONTINUOUS — call 1, call 2, call 3 with
      // model-wait gaps between them. Gating the orb on an in-flight
      // row flashed settled between calls; instead the step stays
      // live while it's the turn's tail (same model as thinking:
      // "live until the next phase's item lands").
      const toolsLive = streaming && isLastStep;
      // Settled span — replayed rows carry ts + rewound startedAt;
      // live rows bank startedAt and stamp ts on finish.
      const elapsedMs =
        !toolsLive && step.startedAt != null && (step.phaseEnd ?? step.endedAt) != null
          ? (step.phaseEnd ?? step.endedAt)! - step.startedAt
          : undefined;
      return (
        <div
          className="flex w-full min-w-0 flex-col items-start gap-2"
        >
          {/* ONE row across the whole phase — mode flips tools →
           *  tools_done only when the next step lands (or the turn
           *  ends), so the orb rides through consecutive calls. The
           *  settled label carries the count and the row IS the
           *  collapse toggle — expanding reveals the chip list. */}
          <AssistantStatus
            mode={toolsLive ? "tools" : "tools_done"}
            thinkingText=""
            startedAt={step.startedAt}
            elapsedMs={elapsedMs}
            doneLabel={`${step.rows.length} 次工具调用`}
            preview={toolActivityPreview(step.rows)}
          >
            <ToolChips
              rows={step.rows}
              renderProse={(text) => (
                <MemoStreamdown text={text} animating={toolsLive} />
              )}
            />
          </AssistantStatus>
        </div>
      );
    }

    if (step.type === "system") {
      return (
        <div
          // `w-full` + `min-w-0` + `overflow-wrap:anywhere` — an
          // upstream error carries a compact JSON blob whose
          // unbreakable tail (the whole `)","param":null,...}` run)
          // otherwise becomes this row's min-content width, pushing
          // it past the column and clipping the tail of every
          // wrapped line. Same fix as the user bubble above.
          className="w-full min-w-0 text-xs text-neutral-500 font-mono py-1 select-none [overflow-wrap:anywhere] break-words"
        >
          {step.text}
        </div>
      );
    }

    if (step.type === "compaction") {
      return (
        <div
          className="w-full min-w-0"
        >
          <CompactionCard
            before={step.before}
            after={step.after}
            removed={step.removed}
            note={step.note}
            pending={step.pending}
            manual={step.manual}
          />
        </div>
      );
    }

    if (step.type === "plan") {
      return (
        <div
          className="w-full min-w-0"
        >
          <PlanCard plan={step.plan} />
        </div>
      );
    }

    return null;
  },
  (a, b) =>
    a.step === b.step &&
    a.turnId === b.turnId &&
    a.stepIdx === b.stepIdx &&
    a.isLastStep === b.isLastStep &&
    a.allowPlain === b.allowPlain &&
    (!a.isLastStep || a.streaming === b.streaming),
);

/** Real step heights, outliving the VirtualStep instance: a turn trimmed and
 *  revealed again, or a session switched away and back, remounts its steps —
 *  an instance-local ref fell back to the length estimate every time, and
 *  each estimate→real correction moved the content above the reader.
 *
 *  Keyed session-first: turn ids are `t{hi}` — an index into ONE history — so
 *  a flat key let session B's `t12` placeholder inherit session A's height
 *  and land the scroll at the wrong offset. The inner map is plain
 *  `stepId → height` (the last measurement, whatever width it was taken at —
 *  a stale-width height still beats the length estimate), so the seed lookup
 *  is O(1) instead of a scan over every cached step.
 *
 *  Bounded: sessions evict oldest-first, and touching a session moves it to
 *  the young end of the outer map (insertion order). */
const stepHeights = new Map<string, Map<string, number>>();
const STEP_HEIGHT_CACHE_MAX = 4000;
let stepHeightEntries = 0;

/** Last measured height for one step in one session, if any. An empty scope
 *  is the unscoped bucket — callers that do not identify a session (tests,
 *  a bare `<ChatStream>` with no `sessionKey`) share it, which is what the
 *  cache did before it was scoped. */
function recallStepHeight(scope: string, id: string): number | undefined {
  return stepHeights.get(scope)?.get(id);
}

function rememberStepHeight(scope: string, id: string, height: number) {
  if (height <= 0) return;
  let bucket = stepHeights.get(scope);
  if (bucket) stepHeights.delete(scope); // re-insert below: touch = youngest
  else bucket = new Map();
  stepHeights.set(scope, bucket);
  if (!bucket.has(id)) stepHeightEntries += 1;
  bucket.set(id, height);
  while (stepHeightEntries > STEP_HEIGHT_CACHE_MAX && stepHeights.size > 1) {
    const oldest = stepHeights.entries().next().value;
    if (oldest === undefined) break;
    stepHeights.delete(oldest[0]);
    stepHeightEntries -= oldest[1].size;
  }
}

/** Keys that scroll the stream — a reader gesture, same as a wheel. */
const SCROLL_KEYS = new Set([
  "ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", " ",
]);

/** The dock pill's glyph pair. At rest it shows what the sidebar HOLDS (a
 *  diff marker, a document box); on hover it swaps to the toggle glyph, so
 *  the button says “you have changes” until the pointer asks “what does
 *  clicking do”. Both live in the same box so nothing shifts. */
function DockGlyph({
  open,
  tab,
}: {
  open: boolean;
  tab?: "changes" | "artifacts" | null;
}) {
  const cls = "h-4 w-4";
  const Content = tab === "artifacts" ? Package : FileDiff;
  const Action = open ? PanelRightClose : PanelRightOpen;
  return (
    <span className="relative flex h-4 w-4 flex-none items-center justify-center">
      <span className="absolute inset-0 flex items-center justify-center transition-opacity duration-150 group-hover:opacity-0">
        <Content className={cls} />
      </span>
      <span className="absolute inset-0 flex items-center justify-center opacity-0 transition-opacity duration-150 group-hover:opacity-100">
        <Action className={cls} />
      </span>
    </span>
  );
}

/** The Dynamic-Island row: the FIRST pill speaks in full, every pill after it
 *  is an icon until the pointer or the keyboard asks for its text. One class
 *  pair so the two can never animate differently. */
const PILL_TEXT = "flex min-w-0 items-center gap-1.5 overflow-hidden transition-all duration-200 ease-out";
const PILL_TEXT_OPEN = "ml-1.5 max-w-[180px] opacity-100";
const PILL_TEXT_COLLAPSED =
  "ml-0 max-w-0 opacity-0 " +
  "group-hover:ml-1.5 group-hover:max-w-[180px] group-hover:opacity-100 " +
  "group-focus-visible:ml-1.5 group-focus-visible:max-w-[180px] group-focus-visible:opacity-100";

/** A pill's text wrapper: `collapsed` ones wait for hover/focus. */
function pillTextClass(collapsed: boolean): string {
  return `${PILL_TEXT} ${collapsed ? PILL_TEXT_COLLAPSED : PILL_TEXT_OPEN}`;
}

/** Overscan around the viewport inside which steps are mounted. */
const STEP_OVERSCAN = 600;

/** Placeholder height before a step has been measured. Text blocks get a
 *  length-based estimate; status rows are ~one collapsed header. The first
 *  mount measures and caches the real height. */
function estimateStepHeight(step: AssistantStep): number {
  switch (step.type) {
    case "text":
      return Math.max(28, Math.min(520, 28 + Math.ceil(step.text.length / 72) * 22));
    case "plan":
      return 104;
    case "compaction":
      return 76;
    case "system":
      return 28;
    case "thinking":
    case "tools":
      return 36;
  }
}

/** A step in the mounted window. Off-window it is a fixed-height placeholder
 *  that still carries the DOM id (rail anchors work); IntersectionObserver
 *  mounts the real TurnStep before it scrolls in and unmounts it after it
 *  leaves the overscan. Expanded tool/thinking state is local to TurnStep,
 *  so leaving the window drops it — the same tradeoff a virtual list makes.
 */
const VirtualStep = memo(function VirtualStep({
  step,
  turnId,
  stepIdx,
  isLastStep,
  streaming,
  scope,
}: {
  step: AssistantStep;
  turnId: string;
  stepIdx: number;
  isLastStep: boolean;
  streaming: boolean;
  /** Session key — the height cache's scope. */
  scope: string;
}) {
  const api = useContext(VirtualStepContext);
  const [mounted, setMounted] = useState(() => typeof IntersectionObserver === "undefined");
  const id = `chat-turn-${turnId}-step-${stepIdx}`;
  const [height, setHeight] = useState(
    () => recallStepHeight(scope, id) ?? estimateStepHeight(step),
  );
  const wrapRef = useRef<HTMLDivElement>(null);
  const measuredRef = useRef(height);
  const unmountTimerRef = useRef<number | null>(null);
  // The live tail never unmounts: a reader scrolled away from a streaming
  // step would otherwise freeze it at a stale height, and the jump back to
  // the bottom lands short.
  const liveRef = useRef(false);
  liveRef.current = streaming && isLastStep;
  // Renderer choice fixed at mount (see TurnStep's text branch).
  const allowPlainRef = useRef(!(streaming && isLastStep));
  if (!mounted) allowPlainRef.current = !(streaming && isLastStep);

  // First layout pass: a step already inside the (final) viewport mounts in
  // this commit, before paint — no blank-placeholder frame. Steps further
  // away keep waiting for the observer, exactly as before.
  const firstCheckRef = useRef(false);
  useLayoutEffect(() => {
    if (mounted || firstCheckRef.current || !api || !wrapRef.current) return;
    firstCheckRef.current = true;
    if (api.isNear(wrapRef.current)) setMounted(true);
  }, [api, mounted]);

  useEffect(() => {
    if (!api || !wrapRef.current) return;
    return api.observe(wrapRef.current, (visible) => {
      if (visible) {
        if (unmountTimerRef.current != null) {
          window.clearTimeout(unmountTimerRef.current);
          unmountTimerRef.current = null;
        }
        setMounted(true);
        return;
      }
      // Hysteresis: a fast scroll must not mount/unmount the same step on
      // every frame. Keep it until it has been out of the overscan for a
      // moment; a re-entry cancels the pending unmount.
      if (unmountTimerRef.current != null) return;
      unmountTimerRef.current = window.setTimeout(() => {
        unmountTimerRef.current = null;
        if (liveRef.current) return;
        // Flush the measured height in the SAME commit as the unmount so
        // swapping the real node for its placeholder cannot move the scroll.
        const measured = wrapRef.current?.offsetHeight ?? 0;
        if (measured > 0) {
          measuredRef.current = measured;
          setHeight(measured);
        }
        setMounted(false);
      }, 1200);
    });
  }, [api]);

  useEffect(
    () => () => {
      if (unmountTimerRef.current != null) window.clearTimeout(unmountTimerRef.current);
    },
    [],
  );

  useLayoutEffect(() => {
    if (!mounted) return;
    const el = wrapRef.current;
    if (!el) return;
    const actual = el.offsetHeight;
    measuredRef.current = actual;
    rememberStepHeight(scope, id, actual);
    const ro = new ResizeObserver(() => {
      const el2 = wrapRef.current;
      if (!el2) return;
      measuredRef.current = el2.offsetHeight;
      rememberStepHeight(scope, id, el2.offsetHeight);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [mounted, id, scope]);

  useEffect(() => {
    if (mounted) return;
    // Freeze the placeholder at the last measured height so unmounting a
    // visible step cannot change the scroll height.
    if (height !== measuredRef.current) setHeight(measuredRef.current);
  }, [mounted, height]);

  return (
    <div
      ref={wrapRef}
      id={id}
      data-step-placeholder={mounted ? undefined : ""}
      data-step-mounted={mounted ? "" : undefined}
      style={mounted ? undefined : { height }}
      className="w-full"
    >
      {mounted && (
        <TurnStep
          step={step}
          turnId={turnId}
          stepIdx={stepIdx}
          isLastStep={isLastStep}
          streaming={streaming}
          allowPlain={allowPlainRef.current}
        />
      )}
    </div>
  );
});

/** Blocks a turn contributes to the mounted budget. A plain user/assistant
 *  turn counts its prompt bubble; a tool-heavy turn counts every step (a
 *  single turn is never split by the cross-turn budget). */
function turnBlockWeight(turn: Turn): number {
  return turn.steps.length + (turn.userText ? 1 : 0);
}

/** Scroll range a column needs before the wheel is worth grabbing. Below it
 *  the stream never emits a scroll event a reader can produce, and the
 *  scroll-up reveal — the only way trimmed turns come back — never fires. */
const MIN_SCROLL_TRAVEL = 240;

/** Back-off before a short column asks again after a page failed to load.
 *  A failure is not "no more history" — but the column emits no scroll
 *  event, so without a timed retry nothing would ever ask again. */
const FILL_RETRY_MS = 2000;

/** Pixels of scroll range, or `null` when the element has no layout at all
 *  (jsdom, a collapsed pane). `null` is not "short": without a real viewport
 *  the trim and reveal rules must stay out of the way rather than guess. */
function scrollTravel(el: HTMLElement): number | null {
  if (el.clientHeight === 0) return null;
  return el.scrollHeight - el.clientHeight;
}

/** One turn's full row — user bubble, assistant steps, turn-end footer.
 * `memo` + the incremental parse cache mean a settled turn's `turn`
 * object survives unchanged across flushes, so a streaming delta only
 * re-renders the LAST turn instead of reconciling the whole history.
 * `streaming` is `view.streaming && isLast` — folded in by the caller so
 * a non-last turn's props stay stable while a turn runs. */
const ChatTurn = memo(function ChatTurn({
  turn,
  isLast,
  retryable,
  streaming,
  scope,
}: {
  turn: Turn;
  isLast: boolean;
  /** Carries the retry control — the last ANSWER turn, which is not always
   *  the structurally last one (a manual-compaction block sits after it). */
  retryable: boolean;
  streaming: boolean;
  /** Session key — the height cache's scope, handed down as a prop rather
   *  than read from context: the stream's first render has no context value
   *  yet, and a placeholder seeded under the wrong scope fell back to the
   *  length estimate on every remount. */
  scope: string;
}) {
  // Undo only reverts the turn's file writes (`undo_last_turn`), so offer it
  // only when a tool row carried a diff — a turn that touched no files has
  // nothing to roll back.
  const hasFileChanges = turn.steps.some(
    (step) =>
      step.type === "tools" &&
      step.rows.some((row) => row.uiType === "diff" || !!row.approval?.diff),
  );
  // Regex-heavy collapse (workspace-file expansions, skill preambles) —
  // once per turn, not once per flush.
  const collapsedUser = useMemo(
    () => (turn.userText ? collapsePromptArtifacts(turn.userText) : ""),
    [turn],
  );
  const copyText = useMemo(() => assistantCopyText(turn), [turn]);
  return (
    <div
      id={`chat-turn-${turn.id}`}
      className="flex w-full flex-col gap-3"
      // Every mounted turn lays out for real. `content-visibility: auto`
      // was tried on this shell and reverted: skipping offscreen layout
      // makes the browser substitute a 320px intrinsic guess for the real
      // height, so (a) `getBoundingClientRect` on a block inside a skipped
      // turn returns a zero rect — the rail's anchor table lost entries and
      // the active mark stopped tracking the scroll — and (b) every
      // estimate→real correction moved the content under the reader. The
      // per-flush cost it was meant to save is already gone (memoized turns
      // + span reuse), so real layout is the cheaper, correct option.
    >
      {turn.userText && (
        <div
          id={`chat-turn-${turn.id}-user`}
          className="ms-auto flex w-fit max-w-[80%] flex-col items-end gap-1"
        >
          {/* `w-fit` sizes the bubble to content, but a single unbroken
              line (a pasted log / URL) inflates `max-content` and pushes the
              bubble past the 80% cap. `max-w-full` clamps it to the wrapper's
              80%, and `min-w-0` on the text lets `overflow-wrap:anywhere`
              actually break mid-word instead of overflowing. */}
          <div className="bg-neutral-100 text-neutral-900 flex w-fit max-w-full flex-col gap-2 rounded-xl px-3.5 py-2.5 text-[14px]">
            <div className="min-w-0 max-w-full text-[14px] leading-relaxed [&_p]:max-w-none [&_p:first-child]:mt-0 [&_p:last-child]:mb-0 select-text [overflow-wrap:anywhere] break-words">
              <UserMemoStreamdown text={collapsedUser} />
            </div>
          </div>
          {turn.ts != null && (
            <TurnFooter align="end" ts={turn.ts} copyText={turn.userText} />
          )}
        </div>
      )}

      {turn.steps.length > 0 && (
        <div
          id={`chat-turn-${turn.id}-assistant`}
          className="flex w-full flex-col items-start gap-2"
        >
          {turn.steps.map((step, stepIdx) => (
            <VirtualStep
              key={`step-${stepIdx}`}
              step={step}
              turnId={turn.id}
              stepIdx={stepIdx}
              isLastStep={stepIdx === turn.steps.length - 1}
              streaming={streaming}
              scope={scope}
            />
          ))}
        </div>
      )}
      {/* ts falls back to the prompt's for text-less turns;
        * hidden while this turn streams. */}
      {turn.steps.length > 0 &&
        !streaming &&
        (turn.assistantTs ?? turn.ts) != null && (
          <div className="-mt-1">
            <TurnFooter
              align="start"
              ts={(turn.assistantTs ?? turn.ts) as number}
              copyText={copyText}
              durationMs={
                turn.assistantTs != null && turn.ts != null && turn.assistantTs > turn.ts
                  ? turn.assistantTs - turn.ts
                  : undefined
              }
              onRetry={retryable ? () => void retryTurn() : undefined}
              onUndo={retryable && hasFileChanges ? () => void undoLastTurn() : undefined}
            />
          </div>
        )}
    </div>
  );
});

export function ChatStream({ view, bottomPad = 128, composerH, loading, sessionKey, hasMore, onLoadOlder, dockOpen, dockCount = 0, dockTab, dockAdds = 0, dockDels = 0, onToggleDock, jobs, jobsOpen, onToggleJobs, subagents }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const endRef = useRef<HTMLDivElement>(null);
  const [virtualApi, setVirtualApi] = useState<VirtualStepApi | null>(null);
  const pinnedRef = useRef(true);
  const [isAtBottom, setIsAtBottom] = useState(true);
  // Index of the last user item seen — a newly appended one means the
  // user just submitted, which always re-pins to the bottom.
  const lastUserIdxRef = useRef<unknown>(null);

  // One observer for every step wrapper. Placeholders keep their DOM id, so
  // the rail's existing target lookup keeps working even before the real
  // TurnStep mounts. Created during the FIRST layout pass (not a passive
  // effect): steps run their own layout effects right after, and the API
  // must already exist for their before-paint `isNear` check.
  useLayoutEffect(() => {
    const root = scrollRef.current;
    if (!root || typeof IntersectionObserver === "undefined") return;
    const callbacks = new Map<Element, (visible: boolean) => void>();
    const io = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) callbacks.get(entry.target)?.(entry.isIntersecting);
      },
      { root, rootMargin: `${STEP_OVERSCAN}px 0px ${STEP_OVERSCAN}px 0px` },
    );
    setVirtualApi({
      observe: (el, onChange) => {
        callbacks.set(el, onChange);
        io.observe(el);
        return () => {
          callbacks.delete(el);
          io.unobserve(el);
        };
      },
      isNear: (el) => {
        // Content-space offset of the step vs the window the reader WILL
        // see: while following the bottom (every open/switch lands there),
        // that is the bottom of the column, not the pre-snap scrollTop.
        const rootRect = root.getBoundingClientRect();
        const r = el.getBoundingClientRect();
        const top = r.top - rootRect.top + root.scrollTop;
        const bottom = top + r.height;
        const viewTop = pinnedRef.current
          ? Math.max(0, root.scrollHeight - root.clientHeight)
          : root.scrollTop;
        return (
          bottom >= viewTop - STEP_OVERSCAN &&
          top <= viewTop + root.clientHeight + STEP_OVERSCAN
        );
      },
    });
    return () => {
      io.disconnect();
      callbacks.clear();
    };
  }, []);

  // Last real scroll gesture (wheel/touch). A snap's own scroll event can
  // be dispatched AFTER more content grew the column — `content-visibility`
  // resolves its 320px intrinsic guesses into real heights, and WebKit has
  // no scroll anchoring to absorb that growth — so the event reads "not at
  // bottom" and used to strand the session mid-list. Rule: a bare scroll
  // event may turn follow-mode ON (at bottom), but only a gesture may turn
  // it OFF. A timer-based hold re-armed by every resize did the opposite —
  // during backfill it never expired and fought the user's own scrolling.
  const gestureAtRef = useRef(0);
  const noteUserScroll = useCallback(() => {
    gestureAtRef.current = performance.now();
  }, []);
  // Keyboard paging (PgUp/arrows/Home/space) scrolls the stream whenever
  // focus is not in a text field — the scroller itself is not focusable, so
  // listen on the document. Without it the bottom pin dragged a keyboard
  // reader back down on the next growth.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!SCROLL_KEYS.has(e.key)) return;
      const t = e.target as HTMLElement | null;
      if (t?.closest("input, textarea, select, [contenteditable='true'], [role='textbox']")) return;
      noteUserScroll();
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, [noteUserScroll]);
  const noteMiddleClick = useCallback(
    (e: ReactPointerEvent) => {
      if (e.button === 1) noteUserScroll();
    },
    [noteUserScroll],
  );
  const snapBottom = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTo({ top: el.scrollHeight, behavior: "instant" });
  }, []);

  useEffect(() => {
    // Session switch must land at the newest message — the scroll
    // container is reused across sessions, so its scrollTop and the
    // pinned flag would otherwise keep whatever position the previous
    // session left (that's why some switches opened at the oldest turn).
    pinnedRef.current = true;
    setIsAtBottom(true);
    lastUserIdxRef.current = null;
    // Stale gestures must not veto the landing: a wheel that happened just
    // before the click (or trackpad momentum) would otherwise let the first
    // "not at bottom" event — caused by our own snap racing intrinsic-size
    // growth — read as "the user is scrolling up", leaving the session
    // stranded mid-list. Only a gesture made AFTER the mount counts.
    gestureAtRef.current = 0;
    requestAnimationFrame(() => {
      snapBottom();
      requestAnimationFrame(snapBottom);
    });
  }, [sessionKey, snapBottom]);

  // Land at the newest message once content actually mounts. The
  // sessionKey rAF above fires against the empty/skeleton container and
  // the [view.items] effect can't reach endRef while `loading` still
  // shows the skeleton — so neither reliably lands the scroll. This
  // fires the moment loading clears with items present — every mounted turn
  // is laid out for real by then, so the snap lands on true heights (the
  // column RO re-pins afterwards for growth added above).
  const [landed, setLanded] = useState(false);
  // Layout effect on purpose: the turns mount with the container at scrollTop
  // 0, and a passive effect runs AFTER the browser has already painted that
  // first frame — the reader sees the top of the transcript (or an empty
  // stretch above the tail) for one frame, then the snap jerks to the newest
  // message. Landing before paint removes that flash.
  useLayoutEffect(() => {
    if (landed || loading || view.items.length === 0) return;
    setLanded(true);
    pinnedRef.current = true;
    setIsAtBottom(true);
    // The mounted tail is laid out for real by this commit, so the snap
    // lands on the true height. One rAF re-snap catches fonts or images
    // settling a frame later — nothing else to chase.
    snapBottom();
    requestAnimationFrame(snapBottom);
  }, [landed, loading, view.items, snapBottom]);

  // ---------- older-history pagination ----------
  // Scroll-near-top fetches the next older page; the prepend grows the
  // column above the viewport, so `restoreRef` carries pre-fetch
  // geometry and the items effect adds back the delta — viewport stays
  // glued to the same message instead of jumping.
  const fetchingRef = useRef(false);
  const [fetching, setFetching] = useState(false);
  const restoreRef = useRef<{ top: number; height: number } | null>(null);
  /** `historyStart` at which the backend answered "no more history" — stops
   *  the short-column fill from re-asking while `hasMore` catches up. */
  const stalledRef = useRef<number | null>(null);
  /** Pending retry after a page failed — see `FILL_RETRY_MS`. */
  const fillRetryRef = useRef(0);

  const maybeLoadOlder = useCallback(async (): Promise<boolean> => {
    const el = scrollRef.current;
    // `!landed` gate: before the mount snap fires, programmatic scrolls
    // (skeleton rAF snap, RO re-pins) can sit under 480px and trigger a
    // fetch whose restore then lands AFTER the landing snap — committing
    // in the same batch and overwriting it, so the session opened at a
    // mid-list anchor instead of the newest message.
    if (!el || !onLoadOlder || !hasMore || !landed || fetchingRef.current) return false;
    fetchingRef.current = true;
    setFetching(true);
    restoreRef.current = { top: el.scrollTop, height: el.scrollHeight };
    let prepended = false;
    try {
      prepended = await onLoadOlder();
    } finally {
      // Empty page → historyStart never moved → the historyStart effect
      // won't consume this restore; drop it so it can't be eaten by the
      // NEXT successful prepend (it would apply a stale viewport delta).
      if (!prepended) restoreRef.current = null;
      fetchingRef.current = false;
      setFetching(false);
    }
    // Return whether a page actually landed: callers must tell "loaded a
    // page" from "bailed out" (no more history / nothing to do).
    return prepended;
  }, [hasMore, onLoadOlder, landed]);

  const scrollToBottom = () => {
    pinnedRef.current = true;
    setIsAtBottom(true);
    isSmoothScrollingRef.current = true;
    const el = scrollRef.current;
    if (el) {
      el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
      setTimeout(() => {
        isSmoothScrollingRef.current = false;
        // Trailing assert — growth during the animation can leave the
        // scroll short of the new bottom; snap whatever's left.
        const el2 = scrollRef.current;
        if (el2) el2.scrollTo({ top: el2.scrollHeight, behavior: "instant" });
        pinnedRef.current = true;
        setIsAtBottom(true);
      }, 450);
    } else {
      endRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
      setTimeout(() => {
        isSmoothScrollingRef.current = false;
      }, 500);
    }
    if (marks.length > 0) {
      activeIdxRef.current = marks.length - 1;
      setActiveMarkId(marks[marks.length - 1].id);
    }
  };

  // Span reuse: stream events only mutate the tail (append / rewrite the
  // last items), and `applyEvent` keeps settled item references stable —
  // `items[i] = {...it}` replaces, never edits in place. Leading spans
  // whose item refs still match are reused wholesale; only the tail
  // turn refolds. A prepend shifts indices but keeps refs — detected via
  // `items[delta] === prev.items[0]`; fold just the new head (extending
  // over the old orphan-head span when the splice lands mid-turn), then
  // reuse every old span at its shifted offset so all existing Turn
  // objects keep their identity — a full refold used to invalidate every
  // ChatTurn memo, re-rendering the entire history on each page fetch.
  const parseCacheRef = useRef<{ items: StreamItem[]; spans: TurnSpan[] } | null>(null);
  const turns = useMemo(() => {
    const items = view.items;
    const spans: TurnSpan[] = [];
    let cursor = 0;
    const prev = parseCacheRef.current;
    if (prev) {
      const delta = items.length - prev.items.length;
      // `delta === items.length` when the previous items were empty —
      // `items[delta]` is out of bounds there, so read it once.
      const boundary = delta > 0 && delta < items.length ? items[delta] : undefined;
      // Pure prepend? `prev.items[0]` resurfacing at index `delta` means
      // the old array shifted right wholesale. Fold only the new head —
      // when the splice lands mid-turn (boundary isn't a `user` item),
      // the head fold extends to swallow the old leading orphan span, so
      // the merged turn refolds exactly as a full fold would produce it.
      let spanIdx = 0;
      if (boundary !== undefined && boundary === prev.items[0]) {
        const headEnd =
          boundary.kind === "user" ? delta : delta + prev.spans[0].span.length;
        foldTurnSpans(items, 0, headEnd, spans);
        cursor = headEnd;
        spanIdx = boundary.kind === "user" ? 0 : 1;
      }
      for (; spanIdx < prev.spans.length; spanIdx++) {
        const s = prev.spans[spanIdx];
        // The last span of the previous fold is the one still growing (appended
        // rows, an assistant item rewritten in place), so its length and item
        // identities no longer describe it. Reusing it handed the tail fold a run
        // of non-user items, and every flush minted another prompt-less turn;
        // refold it from its own start — a turn begins at a user item.
        if (spanIdx === prev.spans.length - 1) break;

        if (cursor + s.span.length > items.length) break;
        let same = true;
        for (let j = 0; j < s.span.length; j++) {
          if (items[cursor + j] !== s.span[j]) {
            same = false;
            break;
          }
        }
        if (!same) break;
        spans.push(s);
        cursor += s.span.length;
      }
    }
    const firstNew = spans.length;
    foldTurnSpans(items, cursor, items.length, spans);
    // The refolded tail is the running turn: keep its unchanged steps (and
    // the turn itself when nothing moved) so only the growing step renders.
    const prevTail = prev?.spans[prev.spans.length - 1]?.turn;
    for (let i = firstNew; i < spans.length; i++) {
      spans[i] = { ...spans[i], turn: reuseTurn(prevTail, spans[i].turn) };
    }
    parseCacheRef.current = { items, spans };
    return spans.map((s) => s.turn);
  }, [view.items]);

  // Tail-first mount: the unveiling commit mounts only the last MOUNT_CHUNK turns
  // (what the viewport shows), then backfills older turns in small macrotask slices,
  // so no commit grows long enough to block input. Growth happens above the pinned
  // bottom, so it stays invisible. A prepended page mounts in full so the
  // scroll-restore delta measures the real added height. Sized for one commit
  // ≈ <300ms in dev, where StrictMode double-renders.
  const MOUNT_CHUNK = 6;
  const BACKFILL_CHUNK = 4;
  // Mount budget while following the bottom. A running session only ever
  // appended turns and kept every one mounted, so layout, the ResizeObserver
  // re-pin and mark measurement all grew with the session's lifetime —
  // where a reopened page (one history page) stayed smooth. The leading
  // turns unmount again (only while pinned: the loss is above the viewport,
  // and the pin re-snaps the bottom); scrolling up reveals them in
  // REVEAL_CHUNK slices before paging history.
  //
  // Turns alone are the wrong unit: a tool-heavy GPT/Claude turn folds a
  // text preamble + tools row per round, so 30 turns can mean 300+ mounted
  // Streamdown/Collapsible subtrees. Keep the newest blocks under a second
  // budget; the turn cap still protects pure Q&A.
  const MAX_MOUNTED = 30;
  const MAX_MOUNTED_BLOCKS = 120;
  const REVEAL_CHUNK = 8;
  /** Leading turns hidden to stay within the block/turn budget. */
  const hiddenByBudget = useMemo(() => {
    let hidden = turns.length;
    let blocks = 0;
    while (hidden > 0) {
      const weight = turnBlockWeight(turns[hidden - 1]);
      // Always mount the newest turn; a single turn is never split by the
      // cross-turn budget. Otherwise stop before this turn would exceed it.
      if (hidden < turns.length && blocks + weight > MAX_MOUNTED_BLOCKS) break;
      blocks += weight;
      hidden--;
    }
    // Never mount more turns than the old cap, even when every one is tiny.
    return Math.max(hidden, turns.length - MAX_MOUNTED);
  }, [turns]);
  const lastTurnCountRef = useRef(turns.length);
  // Leading turns kept unmounted — decremented by backfill ticks to 0.
  // `hiddenTop` only ever SHRINKS: the old tail-anchored `mounted` count
  // let a newly appended turn push the oldest mounted turn out of the
  // slice — it unmounted (big height loss above the viewport) then
  // remounted one tick later (height back), which is exactly the
  // jump-to-bottom-then-fling-upward the mount window was causing.
  const [hiddenTop, setHiddenTop] = useState<number | null>(null);
  // Initial tail-first backfill — off once it reaches the mount budget or
  // the reader takes over (a trim, a reveal), so it never undoes a trim.
  const [backfilling, setBackfilling] = useState(true);
  const [mountedKey, setMountedKey] = useState(sessionKey);
  if (sessionKey !== mountedKey) {
    setMountedKey(sessionKey);
    setHiddenTop(null);
    setBackfilling(true);
    lastTurnCountRef.current = turns.length;
  }
  const lastStartRef = useRef(view.historyStart);
  if (view.historyStart !== lastStartRef.current) {
    lastStartRef.current = view.historyStart;
    // Prepended page mounts in full — the scroll-restore delta must
    // measure the real added height.
    setHiddenTop(0);
    lastTurnCountRef.current = turns.length;
  }
  // Start at the tail-first chunk, but never lower than the block budget
  // (a huge turn/short history can need more leading turns hidden than 6).
  const tailStart = Math.max(0, turns.length - MOUNT_CHUNK);
  if (hiddenTop === null && turns.length > 0) {
    setHiddenTop(Math.max(tailStart, hiddenByBudget));
  }
  const hidden = hiddenTop ?? Math.max(tailStart, hiddenByBudget);
  useEffect(() => {
    if (!loading) loadStamp("content-mounted");
  }, [loading]);

  // Backfill stops at the block/turn budget: a session kept live in memory
  // can hold hundreds of turns, and re-mounting all of them on every switch
  // back is exactly the cost the budget exists to avoid.
  const backfillFloor = hiddenByBudget;
  useEffect(() => {
    if (!backfilling) return;
    // Backfill mounts turns ABOVE the viewport with no scroll compensation —
    // that is only invisible while the bottom pin re-anchors the column. Once
    // the reader has scrolled away, each chunk would push what they are
    // reading down; stop and leave the rest to the scroll-up reveal, which
    // compensates.
    if (hidden <= backfillFloor || (landed && !pinnedRef.current)) {
      setBackfilling(false);
      return;
    }
    // Idle + transition: each chunk's mount is a Streamdown/highlighter-heavy
    // commit — run it in idle time AND as an interruptible transition so
    // a long session's backfill storm never holds the main thread hostage
    // (previously: back-to-back synchronous commits, clicks queued dead).
    const schedule =
      window.requestIdleCallback ??
      ((f: () => void) => window.setTimeout(f, 0));
    const cancel =
      window.cancelIdleCallback ?? ((id: number) => window.clearTimeout(id));
    const id = schedule(() =>
      startTransition(() =>
        setHiddenTop((h) => Math.max(backfillFloor, (h ?? hidden) - BACKFILL_CHUNK)),
      ),
    );
    if (hidden - BACKFILL_CHUNK <= backfillFloor) loadStamp("backfill-done");
    return () => cancel(id as number);
  }, [hidden, backfilling, backfillFloor, landed, isAtBottom]);

  // Trim while following the bottom. Only a NEW turn can grow the settled
  // block total; a live turn's own steps must not evict older turns
  // mid-stream. The turn count guard also keeps a prepend/reveal from being
  // undone.
  useEffect(() => {
    const countChanged = turns.length !== lastTurnCountRef.current;
    if (countChanged) lastTurnCountRef.current = turns.length;
    if (!countChanged || backfilling || loading || !pinnedRef.current) return;
    if (hidden >= hiddenByBudget) return;
    // Trimming never takes the last of the scroll range: a column shorter
    // than the viewport cannot be scrolled, and every route back to older
    // content hangs off a scroll event. `ensureScrollable` owns that case.
    const el = scrollRef.current;
    const travel = el ? scrollTravel(el) : null;
    if (travel !== null && travel < MIN_SCROLL_TRAVEL) return;
    startTransition(() => setHiddenTop(hiddenByBudget));
  }, [turns.length, hidden, hiddenByBudget, backfilling, loading]);

  // Reveal trimmed turns above the viewport, keeping the reader's position:
  // the geometry is captured before the state change and the height the
  // revealed turns add is given back to scrollTop in the same commit.
  const revealRestoreRef = useRef<{ top: number; height: number } | null>(null);
  const revealTrimmed = useCallback((to: number) => {
    const el = scrollRef.current;
    if (el && revealRestoreRef.current === null) {
      revealRestoreRef.current = { top: el.scrollTop, height: el.scrollHeight };
    }
    setBackfilling(false);
    setHiddenTop((h) => Math.min(h ?? to, Math.max(0, to)));
  }, []);
  useLayoutEffect(() => {
    const r = revealRestoreRef.current;
    const el = scrollRef.current;
    revealRestoreRef.current = null;
    if (!r || !el) return;
    el.scrollTop = r.top + (el.scrollHeight - r.height);
  }, [hidden]);

  // The stream must always leave the reader something to scroll with. A column
  // with no scroll range emits no scroll event at all, and BOTH ways older
  // content arrives hang off that event: the trimmed-turn reveal, and the
  // `scrollTop < 480` history fetch. A short column therefore stranded a
  // session on the handful of blocks still mounted — the only escape was the
  // rail, which pages up from a click instead of a scroll.
  const ensureScrollable = useCallback(() => {
    if (loading || !landed || backfilling) return;
    const el = scrollRef.current;
    if (!el) return;
    const travel = scrollTravel(el);
    if (travel === null || travel >= MIN_SCROLL_TRAVEL) return;
    if (hidden > 0) {
      // Locally trimmed turns first: they are already in memory, so this is
      // the cheap way back to a scrollable column.
      revealTrimmed(hidden - REVEAL_CHUNK);
      return;
    }
    if (!hasMore || fetching || fetchingRef.current) return;
    // A page that resolved `false` means the history is drained from this
    // offset. The parent's `hasMore` catches up a render later; until it does
    // the rule must not re-ask, or a drained session would spin on IPC.
    if (stalledRef.current === (view.historyStart ?? 0)) return;
    if (fillRetryRef.current !== 0) return;
    const asked = view.historyStart ?? 0;
    maybeLoadOlder().then(
      (loaded) => {
        if (!loaded) stalledRef.current = asked;
      },
      () => {
        fillRetryRef.current = window.setTimeout(() => {
          fillRetryRef.current = 0;
          ensureScrollableRef.current();
        }, FILL_RETRY_MS);
      },
    );
  }, [
    loading,
    landed,
    backfilling,
    hidden,
    hasMore,
    fetching,
    view.historyStart,
    maybeLoadOlder,
    revealTrimmed,
  ]);
  // The column ResizeObserver is set up once; it reaches the current rule
  // through this ref rather than re-subscribing on every dep change.
  const ensureScrollableRef = useRef(ensureScrollable);
  ensureScrollableRef.current = ensureScrollable;
  useEffect(() => () => window.clearTimeout(fillRetryRef.current), []);
  // A new page (or another session) makes the drain probe meaningful again.
  useEffect(() => {
    stalledRef.current = null;
  }, [view.historyStart, sessionKey]);
  // Runs on every input the rule reads (a trim, a landing, a page arriving),
  // and on the geometry changes those deps cannot see — a window resize, a
  // first layout after the window was hidden, the dock opening. `resize` and
  // the column ResizeObserver below both call it for that half.
  useLayoutEffect(() => {
    ensureScrollable();
  }, [ensureScrollable]);
  useEffect(() => {
    window.addEventListener("resize", ensureScrollable);
    return () => window.removeEventListener("resize", ensureScrollable);
  }, [ensureScrollable]);

  const visibleTurns = hidden > 0 ? turns.slice(hidden) : turns;

  // The turn the retry/copy footer belongs to. A manual-compaction block is
  // its own turn at the tail, but retrying is about the last ANSWER — the
  // button must stay on the previous turn instead of moving onto the card.
  const lastAnswerIdx = (() => {
    for (let i = visibleTurns.length - 1; i >= 0; i--) {
      const t = visibleTurns[i];
      if (t.userText || t.steps.some((s) => s.type !== "compaction")) return i;
    }
    return visibleTurns.length - 1;
  })();

  // Only items in the CURRENT turn count — a stale active item left by a
  // dead earlier turn (streaming assistant that never got AssistantMessage,
  // unresolved approval) would otherwise suppress "正在回复" forever while
  // its own indicator sits scrolled-away in the old turn.
  const lastUserIdx = view.items.reduce(
    (acc, it, i) => (it.kind === "user" ? i : acc),
    -1
  );
  const hasActiveItem = view.items.slice(lastUserIdx + 1).some((i) => {
    if (i.kind === "thinking" && !i.done) return true;
    if (i.kind === "assistant" && i.streaming) return true;
    if (i.kind === "tool" && i.content === undefined) return true;
    if (i.kind === "approval" && !i.resolved) return true;
    return false;
  });

  const showReplyWait = view.streaming && !hasActiveItem;

  // A compaction pass has no user turn to anchor its clock (a manual
  // `/compact` arrives between turns) — the running card's own stamp is the
  // phase start.
  const pendingCompactionAt = view.items.reduce<number | undefined>(
    (acc, it) => (it.kind === "compaction" && it.pending ? it.ts : acc),
    undefined
  );
  const compacting = view.state === "Compacting";

  const marks = useMemo<RailMark[]>(() => {
    if (turns.length === 0) return [];

    const list: RailMark[] = [
      {
        id: "mark-top",
        type: "top",
        targetId: "chat-stream-top",
        previewTitle: "回到顶部",
        previewSnippet: "",
      },
    ];

    // Unloaded-history placeholders — the rail is an overview of the
    // WHOLE session, so turns above the loaded page still occupy their
    // share as faded dots (click pages up to that region). Two rows per
    // unloaded turn: the loaded side spends a user mark AND an assistant
    // mark per turn, so one row per unloaded turn made the unloaded share
    // read as half its real length and put the load boundary in the wrong
    // place on the strip.
    const { unloaded, boundaryTurn } = railHistoryBoundary(turns, view);
    // A real half-loaded answer is missing one prompt; standalone notices
    // and fully loaded views contribute no phantom placeholder.
    const openTurns = Math.max(0, unloaded - (boundaryTurn ? 1 : 0));
    const PH_CAP = 120;
    const boundaryRows = boundaryTurn ? 1 : 0;
    const openRows = Math.max(0, Math.min(openTurns * 2, PH_CAP - boundaryRows));
    for (let i = 0; i < openRows; i++) {
      // Exact two-rows-per-turn pairing while the range fits; even mapping
      // back onto the real ordinal range once the cap compresses it.
      const phIndex =
        openRows === openTurns * 2
          ? Math.floor(i / 2)
          : Math.round((i / Math.max(1, openRows - 1)) * Math.max(0, openTurns - 1));
      list.push({
        id: `mark-ph-${i}`,
        type: "placeholder",
        phIndex,
        targetId: "",
        previewTitle: "未加载的历史",
        previewSnippet: `第 ${phIndex + 1} 个回合`,
      });
    }
    if (boundaryTurn) {
      list.push({
        id: "mark-ph-boundary",
        type: "placeholder",
        // Its prompt is ordinal `unloaded - 1`; clicking lands on the
        // answer below because that is the half that is actually loaded.
        phIndex: Math.max(0, unloaded - 1),
        targetId:
          boundaryTurn.steps.length > 0
            ? `chat-turn-${boundaryTurn.id}-assistant`
            : "",
        previewTitle: "未加载的历史",
        // `turnOffset` counts the boundary turn as unloaded, so its prompt is
        // 1-based `unloaded`.
        previewSnippet: `第 ${Math.max(1, unloaded)} 个回合的提问`,
      });
    }

    // Turn numbers are GLOBAL ordinals, not loaded indices: on a reopened
    // session the first loaded turn is turn `unloaded + 1` — or `unloaded`
    // when the page cut it in half — so "#1" was naming a turn the reader
    // was not looking at. Only a turn WITH a prompt advances the ordinal:
    // the page-boundary half-turn and a manual-compaction block have no user
    // turn of their own, and counting them shifted every later "#N". A steer
    // bubble keeps its mark but stays inside the turn it interrupted.
    let ordinal = unloaded;
    turns.forEach((turn, idx) => {
      if (opensTurn(turn)) ordinal += 1;
      const turnNum = ordinal;
      // Keep marks for locally trimmed turns too: the rail is the session
      // overview, and handleSelectMark reveals the turn before landing on a
      // step. Skipping them made the left rail lose anchors past the mount
      // budget.
      if (turn.userText) {
        // Length from the COLLAPSED preview, not the raw text: a `/skill` or
        // `@file` mention expands into a multi-kilobyte preamble that the
        // bubble never shows, and measuring that would peg every such prompt
        // at full width.
        const userPreview = userPreviewOf(turn);
        list.push({
          id: `mark-${turn.id}-user`,
          type: "user",
          turnId: turn.id,
          targetId: `chat-turn-${turn.id}-user`,
          previewTitle: turn.steer ? `用户插话 · 第 ${Math.max(1, turnNum)} 轮` : `用户提问 #${turnNum}`,
          previewSnippet: clipPreview(userPreview),
          len: userPreview.length,
        });
      }

      if (turn.steps.length > 0) {
        const isLastTurn = idx === turns.length - 1;
        // One mark per block, not per turn: a turn is the prompt plus every
        // block the answer arrived in (thinking / tools / prose), and a single
        // dash per turn flattened agentic turns into stubs. A long agentic
        // answer, though, ran to dozens of identical dashes between two
        // prompts, so past MAX_STEP_MARKS consecutive steps share one dash
        // (it lands on the first step of its run).
        const n = turn.steps.length;
        const groups = Math.min(n, MAX_STEP_MARKS);
        for (let g = 0; g < groups; g++) {
          const from = Math.floor((g * n) / groups);
          const to = Math.floor(((g + 1) * n) / groups);
          const step = turn.steps[from];
          const isStreaming = view.streaming && isLastTurn && to === n;
          let len = 0;
          for (let k = from; k < to; k++) len += stepMarkInfo(turn.steps[k]).weight;
          const extra = to - from - 1;
          list.push({
            id: `mark-${turn.id}-step-${from}`,
            type: "assistant",
            turnId: turn.id,
            targetId: `chat-turn-${turn.id}-step-${from}`,
            previewTitle:
              (step.type === "compaction"
                ? `上下文压缩${step.manual ? "（手动）" : "（自动）"}${
                    step.pending ? " · 进行中" : ""
                  }`
                : step.type === "system"
                  ? "系统提醒"
                  : `助手回复 #${turnNum} · ${STEP_LABEL[step.type]}`) +
              (extra > 0 ? ` 等 ${extra + 1} 段` : "") +
              (isStreaming ? " (生成中)" : ""),
            previewSnippet: stepMarkInfo(step).preview,
            len,
            isStreaming,
          });
        }
      } else if (idx === turns.length - 1 && (view.streaming || showReplyWait)) {
        list.push({
          id: `mark-${turn.id}-assistant-waiting`,
          type: "assistant",
          turnId: turn.id,
          targetId: "chat-reply-wait",
          previewTitle: `助手回复 #${turnNum}`,
          previewSnippet: "正在等待回复…",
          isStreaming: true,
        });
      }
    });

    // Last mark: `handleSelectMark`'s is-last branch jumps to the newest message.
    list.push({
      id: "mark-end",
      type: "end",
      targetId: "chat-stream-end",
      previewTitle: "跳到最新",
      previewSnippet: "",
    });

    return list;
  }, [turns, view.streaming, showReplyWait, view.historyStart, view.turnOffset, view.turnTotal]);

  // Preview text changes on every delta; geometry only changes when the
  // rendered content grows or marks mount/unmount. Keep the observers and
  // measurement callbacks stable while their inputs follow the latest render.
  const marksRef = useRef(marks);
  marksRef.current = marks;
  const markTargets = marks.map((mark) => `${mark.id}:${mark.targetId}`).join("\n");
  const measureMarks = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const containerTop = el.getBoundingClientRect().top;
    const base = el.scrollTop - containerTop;
    const tops: { id: string; top: number }[] = [];
    let prev = -Infinity;
    for (const mark of marksRef.current) {
      if (mark.type === "top") continue;
      const target = document.getElementById(mark.targetId);
      if (!target) continue;
      const rect = target.getBoundingClientRect();
      // A block inside a `content-visibility: auto` subtree that is
      // currently skipped has NO layout box — its rect is all zeros, which
      // would land in `tops` as a bogus ≈scrollTop value and break the
      // monotonic order the active-mark binary search depends on (that is
      // exactly how the rail ended up highlighting the wrong mark).
      if (rect.width === 0 && rect.height === 0) continue;
      // Windowed rebuilds can still measure marginally out of order; clamp
      // so the array stays sorted no matter what.
      const top = Math.max(rect.top + base, prev);
      prev = top;
      tops.push({ id: mark.id, top });
    }
    markTopsRef.current = tops;
  }, []);

  const appliedStartRef = useRef(view.historyStart);
  // Layout effect, like the trimmed-turn reveal: the prepend commits inside a
  // transition, whose passive effects run AFTER paint — one frame showed the
  // new page's top before the restore pulled the reader back.
  useLayoutEffect(() => {
    // Consume the restore geometry ONLY when a page actually prepended
    // (historyStart moved). Before this guard, any streaming delta that
    // landed while the page IPC was in flight ate the restore — the real
    // prepend then mounted with no compensation and jumped the viewport.
    if (view.historyStart === appliedStartRef.current) return;
    appliedStartRef.current = view.historyStart;
    const r = restoreRef.current;
    const el = scrollRef.current;
    restoreRef.current = null;
    // `!landed`: a restore captured off the skeleton would overwrite the
    // landing snap — this effect runs AFTER the landing effect in a
    // batched commit, and the newest message is the right anchor until
    // the user has actually landed anyway. Consume + drop it instead.
    if (!r || !el || !landed) return;
    el.scrollTop = r.top + (el.scrollHeight - r.height);
    measureMarks();
  }, [view.historyStart, view.items, measureMarks, landed]);

  const [activeMarkId, setActiveMarkId] = useState<string>("mark-top");
  /** Index of `activeMarkId` inside `marks` — `updateActiveMark` starts its
   *  walk here, so a scroll frame measures one or two marks instead of all. */
  const activeIdxRef = useRef(0);

  const isSmoothScrollingRef = useRef(false);

  // Absolute scroll-content offsets per mark — measured ONCE after each
  // layout commit, not per scroll event. The old version did N×
  // getElementById + getBoundingClientRect inside every scroll callback
  // (2 marks per turn → hundreds of forced layout reads per frame).
  const markTopsRef = useRef<{ id: string; top: number }[]>([]);
  // Fresh view handle for the placeholder seek loop — `onLoadOlder`'s
  // resolution precedes the prop update by a render.
  const viewRef = useRef(view);
  viewRef.current = view;
  const turnsRef = useRef(turns);
  turnsRef.current = turns;
  const updateActiveMark = useCallback(() => {
    if (isSmoothScrollingRef.current) return;
    const el = scrollRef.current;
    const marks = marksRef.current;
    if (!el || marks.length === 0) return;

    const { scrollTop, scrollHeight, clientHeight } = el;

    // Bottom first: when the loaded slice roughly fits the viewport both
    // branches match, and the newest message is the honest description of
    // what is on screen — "top" was highlighting the first mark there.
    if (scrollHeight - scrollTop - clientHeight < 48) {
      activeIdxRef.current = marks.length - 1;
      setActiveMarkId(marks[marks.length - 1].id);
      return;
    }

    if (scrollTop < 32) {
      // Very top of the loaded slice. When the history above it is still
      // unloaded, that boundary IS the last placeholder dot — pointing at
      // "回到顶部" there read as "scrolled into the load region and the
      // rail jumped to the top".
      const firstLoaded = marks.findIndex(
        (m) => m.type !== "top" && m.type !== "placeholder",
      );
      const hasPlaceholders = marks.some((m) => m.type === "placeholder");
      const idx = hasPlaceholders && firstLoaded > 0 ? firstLoaded - 1 : 0;
      activeIdxRef.current = idx;
      setActiveMarkId(marks[idx].id);
      return;
    }

    // Walk out from the mark highlighted last, measuring as we go, and stop
    // on the last mark whose top sits above the trigger line. Positions are
    // read HERE, in the scroll frame, not from a cached table: a turn's height
    // changes outside an items update all the time (an expanded tool row, a
    // late image, a font swap), and a cached table then points at the wrong
    // mark — or at marks that no longer exist — so the rail sat on a highlight
    // that had nothing to do with the viewport. A couple of layout reads per
    // frame is the price of always being right.
    const trigger = scrollTop + 100;
    const containerTop = el.getBoundingClientRect().top;
    const topOf = (m: RailMark): number | null => {
      if (!m.targetId) return null; // placeholder — never in the DOM
      const node = document.getElementById(m.targetId);
      if (!node) return null; // windowed out
      const rect = node.getBoundingClientRect();
      if (rect.width === 0 && rect.height === 0) return null;
      return rect.top - containerTop + scrollTop;
    };
    let idx = Math.min(Math.max(activeIdxRef.current, 0), marks.length - 1);
    while (idx + 1 < marks.length) {
      const next = topOf(marks[idx + 1]);
      if (next === null) {
        idx++; // no box to compare (a placeholder) — step over it
        continue;
      }
      if (next <= trigger) idx++;
      else break;
    }
    while (idx > 0) {
      const cur = topOf(marks[idx]);
      if (cur === null) {
        idx--;
        continue;
      }
      if (cur > trigger) idx--;
      else break;
    }
    activeIdxRef.current = idx;
    setActiveMarkId(marks[idx].id);
  }, []);

  // Measure structural changes here; ResizeObserver covers text growth,
  // expanding a thinking/tool row, images, and fonts. A folded reasoning
  // delta has no geometry to measure.
  useEffect(() => {
    measureMarks();
    updateActiveMark();
  }, [measureMarks, updateActiveMark, markTargets, hidden, loading]);

  useEffect(() => {
    const el = scrollRef.current;
    const col = el?.firstElementChild;
    if (!el || !col) return;
    let debounce = 0;
    let pinFrame = 0;
    let fillFrame = 0;
    const ro = new ResizeObserver(() => {
      // Growth below the viewport → re-anchor the bottom while following.
      // The pin's own scroll events can't clear follow-mode anymore (only
      // a real gesture can), so the landing converges frame by frame. One
      // re-pin per frame: a window drag reflows the column on every event.
      if (pinnedRef.current && !pinFrame) {
        pinFrame = requestAnimationFrame(() => {
          pinFrame = 0;
          if (pinnedRef.current) el.scrollTo({ top: el.scrollHeight, behavior: "instant" });
        });
      }
      // A column or viewport that changed height without the window resizing
      // (first layout after a hidden window, the dock opening) may have
      // dropped under the scroll range the reader needs.
      if (!fillFrame) {
        fillFrame = requestAnimationFrame(() => {
          fillFrame = 0;
          ensureScrollableRef.current();
        });
      }
      // A layout change moves every mark below it: re-measure AND re-derive
      // the highlight, otherwise the rail keeps pointing at offsets that no
      // longer exist until the reader happens to scroll again.
      window.clearTimeout(debounce);
      debounce = window.setTimeout(() => {
        measureMarks();
        updateActiveMark();
      }, 250);
    });
    ro.observe(col);
    // The viewport too: the dock shrinks the scroller, not the column.
    ro.observe(el);
    return () => {
      window.clearTimeout(debounce);
      cancelAnimationFrame(pinFrame);
      cancelAnimationFrame(fillFrame);
      ro.disconnect();
    };
  }, [measureMarks, updateActiveMark]);

  const lastUserKey = useMemo(() => {
    for (let i = view.items.length - 1; i >= 0; i--) {
      const item = view.items[i];
      if (item.kind === "user") return item.hi ?? item;
    }
    return null;
  }, [view.items]);
  // `historyStart` / session the last key was read against — tells a page
  // prepended above from a prompt appended below.
  const keyStartRef = useRef({ start: view.historyStart, session: sessionKey });
  useEffect(() => {
    // A fresh prompt re-pins; history prepends preserve the existing key.
    // Stream growth follows through ResizeObserver instead of forcing
    // scrollIntoView (and layout) on every reasoning delta.
    //
    // Except when the loaded slice had NO user row yet: a tool-heavy last
    // turn can fill the whole first page, so the key starts null and the
    // first older page brings that turn's prompt in. That key change comes
    // from ABOVE (same session, historyStart moved back) — not a new prompt;
    // re-pinning there yanked the reader to the bottom on the first scroll
    // up after launch.
    const prev = keyStartRef.current;
    const prepended =
      prev.session === sessionKey &&
      prev.start != null &&
      view.historyStart != null &&
      view.historyStart < prev.start;
    keyStartRef.current = { start: view.historyStart, session: sessionKey };
    if (lastUserKey !== null && lastUserKey !== lastUserIdxRef.current && !prepended) {
      pinnedRef.current = true;
      endRef.current?.scrollIntoView({ block: "end" });
    }
    lastUserIdxRef.current = lastUserKey;
    const marks = marksRef.current;
    if (pinnedRef.current) {
      setIsAtBottom(true);
      if (marks.length > 0) {
        activeIdxRef.current = marks.length - 1;
        setActiveMarkId(marks[marks.length - 1].id);
      }
    }
  }, [lastUserKey, markTargets, sessionKey, loading, view.historyStart]);

  const markRafRef = useRef(0);
  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    if (el.scrollTop < 480) {
      // Locally trimmed turns come back first; history pages only once
      // everything loaded is mounted again.
      // Same `landed` gate as paging: before the landing snap, the skeleton's
      // scrollTop 0 is not the reader scrolling up. Never while pinned either
      // — the trim owns that state, and a short column would trade turns
      // back and forth between the two.
      if (hidden > 0) {
        if (landed && !pinnedRef.current && revealRestoreRef.current === null) {
          revealTrimmed(hidden - REVEAL_CHUNK);
        }
      } else void maybeLoadOlder();
    }
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
    // Programmatic smooth scrolls emit intermediate events far from the
    // bottom — letting them rewrite pinnedRef kills follow-mode mid-
    // animation, and content growth during the animation then strands
    // the scroll short of the new bottom with the pin dead.
    if (!isSmoothScrollingRef.current) {
      if (atBottom) pinnedRef.current = true;
      else if (performance.now() - gestureAtRef.current < 400) {
        // A wheel/touch just happened → the user is really scrolling up.
        pinnedRef.current = false;
      }
    }
    setIsAtBottom(pinnedRef.current);
    // One mark update per frame max — scroll events fire faster than
    // frames, and the rail highlight doesn't need event-rate precision.
    if (!markRafRef.current) {
      markRafRef.current = requestAnimationFrame(() => {
        markRafRef.current = 0;
        updateActiveMark();
      });
    }
  };

  const handleSelectMark = (mark: RailMark) => {
    if (mark.type === "top") {
      pinnedRef.current = false;
      setIsAtBottom(false);
      activeIdxRef.current = 0;
      setActiveMarkId(mark.id);
      isSmoothScrollingRef.current = true;
      scrollRef.current?.scrollTo({ top: 0, behavior: "smooth" });
      setTimeout(() => {
        isSmoothScrollingRef.current = false;
      }, 500);
      return;
    }

    if (mark.type === "placeholder") {
      // Show the pick immediately: paging up takes a moment per page, and
      // without this the rail kept highlighting whatever mark was active while
      // the click looked like it had done nothing at all.
      pinnedRef.current = false;
      setIsAtBottom(false);
      activeIdxRef.current = marks.indexOf(mark);
      setActiveMarkId(mark.id);
      // The boundary dot's own prompt sits above the slice, but the answer
      // it belongs to is already mounted right below it — land there
      // instead of paging up to a turn whose tail is what this dot stands
      // for (paging would also be wrong: the prompt it names can only
      // arrive together with the answer that is already here).
      if (mark.targetId) {
        const mounted = document.getElementById(mark.targetId);
        if (mounted) {
          isSmoothScrollingRef.current = true;
          mounted.scrollIntoView({ behavior: "smooth", block: "start" });
          setTimeout(() => {
            isSmoothScrollingRef.current = false;
          }, 500);
          return;
        }
      }
      // Page up until the dot's OWN turn is loaded, then scroll to it.
      // With `turnOffset` the ordinal is exact: an accumulated slice's
      // user turns are ordinals `turnOffset, turnOffset+1, …` in order, so
      // the target is the `phIndex − turnOffset`-th user turn. Without the
      // field (a view built before it existed) this falls back to
      // estimating a message index from the average turn length.
      const ordinal = mark.phIndex ?? 0;
      const loadedUser = turnsRef.current.reduce((n, t) => n + (opensTurn(t) ? 1 : 0), 0);
      const unloadedNow = Math.max(0, (viewRef.current.turnTotal ?? loadedUser) - loadedUser);
      const start = viewRef.current.historyStart ?? 0;
      const approx = Math.round((ordinal + 0.5) * (start / Math.max(1, unloadedNow)));
      void (async () => {
        // Page up until the dot's region is actually in. The old loop had
        // two traps: a blind 12-iteration cap that gave up mid-way — after
        // which the target lookup fell back to the topmost loaded turn,
        // reading as "clicked an unloaded dot, jumped to the top" — and a
        // bare `setTimeout(0)` wait that burned those iterations in
        // milliseconds whenever a page was already in flight. Now it waits
        // out an in-flight page, stops only at real end conditions, and
        // lets the prepend's transition commit before picking the target.
        const nextFrame = () =>
          new Promise<void>((r) => {
            const t = window.setTimeout(r, 60);
            requestAnimationFrame(() => {
              window.clearTimeout(t);
              r();
            });
          });
        for (let i = 0; i < 400; i++) {
          const cur = viewRef.current;
          const hs = cur.historyStart ?? 0;
          const inYet = cur.turnOffset != null ? cur.turnOffset <= ordinal : hs <= approx;
          if (inYet || hs <= 0) break; // region in / session start reached
          if (fetchingRef.current) {
            await nextFrame(); // a page is already loading — let it land
            continue;
          }
          // maybeLoadOlder carries the fetching guard + scroll-restore
          // bookkeeping — calling onLoadOlder directly could double-fetch.
          const prepended = await maybeLoadOlder();
          if (!prepended && !fetchingRef.current) break; // history ended
          await nextFrame();
        }
        await nextFrame();
        const loaded = turnsRef.current;
        let target: (typeof loaded)[number] | undefined;
        const off = viewRef.current.turnOffset;
        if (off != null) {
          const userTurns = loaded.filter(opensTurn);
          const idx = Math.min(Math.max(ordinal - off, 0), userTurns.length - 1);
          target = userTurns[idx] ?? loaded[0];
        } else {
          target = loaded.find((t) => t.hi != null && t.hi >= approx)
            ?? loaded[loaded.length - 1];
        }
        const el = target ? document.getElementById(`chat-turn-${target.id}`) : null;
        if (el) {
          pinnedRef.current = false;
          setIsAtBottom(false);
          isSmoothScrollingRef.current = true;
          el.scrollIntoView({ behavior: "smooth", block: "start" });
          setTimeout(() => {
            isSmoothScrollingRef.current = false;
          }, 500);
        }
      })();
      return;
    }

    const isLast = marks[marks.length - 1]?.id === mark.id;
    pinnedRef.current = isLast;
    setIsAtBottom(isLast);
    activeIdxRef.current = marks.indexOf(mark);
    setActiveMarkId(mark.id);

    if (isLast) {
      isSmoothScrollingRef.current = true;
      endRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
      setTimeout(() => {
        isSmoothScrollingRef.current = false;
        // Growth during the animation (intrinsic-size guesses resolving)
        // can leave the smooth scroll short of the real bottom — snap the
        // remainder, same as the scroll-to-bottom button does.
        snapBottom();
      }, 500);
      return;
    }

    const el = document.getElementById(mark.targetId);
    if (el) {
      const targetId = mark.targetId;
      isSmoothScrollingRef.current = true;
      el.scrollIntoView({ behavior: "smooth", block: "start" });
      setTimeout(() => {
        isSmoothScrollingRef.current = false;
        // Virtualized steps mount below the anchor while the smooth scroll
        // runs; snap once at the end so the mark lands on the step, not on
        // the pre-mount placeholder offset.
        document.getElementById(targetId)?.scrollIntoView({ block: "start" });
      }, 500);
      return;
    }
    // Trimmed off the mounted window — mount down to its turn, then land.
    const turnIdx = mark.turnId ? turns.findIndex((t) => t.id === mark.turnId) : -1;
    if (turnIdx >= 0 && turnIdx < hidden) {
      revealTrimmed(turnIdx);
      requestAnimationFrame(() => {
        const target = document.getElementById(mark.targetId);
        if (!target) return;
        isSmoothScrollingRef.current = true;
        target.scrollIntoView({ behavior: "smooth", block: "start" });
        setTimeout(() => {
          isSmoothScrollingRef.current = false;
        }, 500);
      });
      return;
    }
    // Windowed out — scroll to the shell's measured position; the IO
    // margin mounts the content before it enters the viewport.
    const t = markTopsRef.current.find((x) => x.id === mark.id)?.top;
    if (t != null && scrollRef.current) {
      isSmoothScrollingRef.current = true;
      scrollRef.current.scrollTo({ top: t - 24, behavior: "smooth" });
      setTimeout(() => {
        isSmoothScrollingRef.current = false;
      }, 500);
    }
  };


  // ---- Right-click menu -------------------------------------------------
  // The native context menu is suppressed app-wide; the stream gets its own
  // so a selected passage can be copied or handed to the composer as a
  // quote. `selRef` mirrors the selection captured at menu-open time —
  // reading `window.getSelection()` inside the item callback can race the
  // browser clearing it.
  const selRef = useRef("");
  const [hasSelection, setHasSelection] = useState(false);
  const streamRootRef = useRef<HTMLDivElement>(null);

  const syncSelection = () => {
    const text = selectionWithin(streamRootRef.current);
    selRef.current = text;
    setHasSelection(text.length > 0);
  };

  const copySelection = async () => {
    const text = selRef.current;
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      // WebKit fallback when the async clipboard API is unavailable.
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      try {
        document.execCommand("copy");
      } catch {
        toast.error("复制失败");
      }
      document.body.removeChild(ta);
    }
  };

  const askAboutSelection = () => {
    const text = selRef.current.trim();
    if (!text) return;
    requestQuote(text);
    // Collapse the highlight once the quote is handed off — the composer
    // echo is now the canonical view of it.
    window.getSelection()?.removeAllRanges();
    setHasSelection(false);
  };

  const pasteToComposer = async () => {
    try {
      const clip = await navigator.clipboard.readText();
      if (clip) requestQuote(clip);
    } catch {
      toast.error("无法读取剪贴板");
    }
  };

  // Turn rows are memoized as a whole — mark-highlight changes and
  // at-bottom toggles during scroll then skip the entire subtree
  // instead of re-rendering hundreds of turn divs per crossing.

  // Whether the floating row above the composer carries the dock pill — the
  // jump-to-bottom arrow lays itself out differently in the two cases.
  //
  // The pill speaks for THIS session: it exists because the session produced
  // files, never because the surrounding repository happens to be dirty. The
  // working tree's own state is the title bar chip's business.
  const dockPillVisible = !!onToggleDock && dockCount > 0;
  const jobsPillVisible = !!onToggleJobs && !!jobs;
  // The subagents' capsule: it says how many children are out and names their
  // tasks on demand, since a running child has no row of its own in the dock.
  const subagentPill = subagents && subagents.length > 0 ? summarizeChildren(subagents) : null;
  const subagentPillVisible = !!subagentPill;
  // One capsule speaks, the ones behind it collapse (the island's rule). The
  // dock pill leads whenever it is up, so the jobs pill beside it is an orb
  // until the pointer asks for the count. A third pill would follow the same
  // rule: collapsed only because something already leads it.
  const jobsCollapsed = dockPillVisible;
  const subagentCollapsed = dockPillVisible || jobsPillVisible;
  // The arrow yields to whichever pills are up — one rule for the row.
  const anyPill = dockPillVisible || jobsPillVisible || subagentPillVisible;

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div
          ref={streamRootRef}
          onContextMenu={syncSelection}
          className="bg-white flex-1 min-h-0 w-full flex flex-col select-text relative"
        >
      <ChatTurnRail
        marks={marks}
        activeId={activeMarkId}
        onSelectMark={handleSelectMark}
        loading={loading}
      />

      <div
        className="stream-scroll"
        ref={scrollRef}
        onScroll={onScroll}
        onWheel={noteUserScroll}
        onTouchStart={noteUserScroll}
        // Middle-click autoscroll is a reader scroll too (keyboard paging
        // is caught at document level — see the SCROLL_KEYS effect).
        onPointerDown={noteMiddleClick}
      >
        {/* pl-11 reserves the rail lane — only once the rail renders
            (content or loading); an empty session keeps symmetric
            padding so the greeting stays centred. */}
        <div
          className={cn(
            "max-w-3xl w-full mx-auto pr-4 pt-4 flex shrink-0 flex-col gap-4 min-h-full",
            loading || view.items.length > 0 ? "pl-11" : "pl-4",
          )}
        >
          {loading ? (
            // Loading = skeleton INSTEAD of content — the old
            // session's turns must never linger under it.
            <ChatSkeleton />
          ) : (
            <>
              {view.items.length === 0 && !view.streaming && (
                <EmptyGreeting />
              )}

              {hasMore &&
                (fetching ? (
                  // Skeleton rows stand in for the incoming page — reads
                  // as content resolving above the fold, not a label.
                  <OlderPageSkeleton />
                ) : (
                  <div className="flex justify-center py-1" aria-live="polite">
                    <span className="text-[11px] text-muted-foreground/60">
                      继续上滑加载更早的消息
                    </span>
                  </div>
                ))}

              <VirtualStepContext.Provider value={virtualApi}>
                {visibleTurns.map((turn, turnIdx) => {
                  const isLast = turnIdx === visibleTurns.length - 1;
                  return (
                    <ChatTurn
                      key={turn.id}
                      turn={turn}
                      isLast={isLast}
                      retryable={turnIdx === lastAnswerIdx}
                      streaming={view.streaming && isLast}
                      scope={sessionKey ?? ""}
                    />
                  );
                })}
              </VirtualStepContext.Provider>

            {/* Turn-running clock — visible for the WHOLE streaming turn, not
                just the pre-first-delta wait. Anchored on `view.turnStartedAt`
                (stamped on UserPrompt) so it reads as "this turn has been
                running Xs" and survives a session switch; the per-phase rows
                above still show their own thinking/tools durations. */}
            {view.streaming && (view.turnStartedAt != null || compacting) && (
              <div id="chat-reply-wait">
                <AssistantStatus
                  mode={compacting ? "compacting" : "reply"}
                  thinkingText=""
                  startedAt={compacting ? pendingCompactionAt : view.turnStartedAt}
                />
              </div>
            )}

            {/* Dedicated bottom block spacer: guarantees that when scrolled to the bottom,
                the latest message and status indicator are never blocked by the floating composer card */}
            <div
              style={{ height: Math.max(bottomPad, 180) }}
              className="w-full flex-none pointer-events-none select-none"
              aria-hidden="true"
            />
            <div id="chat-stream-end" ref={endRef} />
            </>
          )}
        </div>
      </div>

      {/* One centered floating row above the composer, two behaviours:
          · with a pill up (the jobs pill, the subagents capsule, the dock
            pill, or several side by side), `width`/`margin` animate — the row
            grows to the right and the pills slide left to make room for the
            arrow, which fades in on the right (an opacity-only swap would leave a
            40px hole beside a centred pill);
          · with no pill, the arrow IS the row, so it keeps its own width and
            only fades, rising out of the composer's edge; a growing margin
            would make a lone button drift in from the right. */}
      <div
        style={{ bottom: `${(composerH ?? (bottomPad ? bottomPad - 24 : 140)) + 12}px` }}
        className="absolute left-1/2 z-30 flex -translate-x-1/2 items-center"
      >
        {dockPillVisible && (
          <TooltipSimple
            content={
              dockOpen
                ? `关闭侧栏（${dockTab === "artifacts" ? "产物" : "改动"} ${dockCount}）`
                : `打开侧栏 · ${dockTab === "artifacts" ? "产物" : "改动"} ${dockCount}`
            }
            side="top"
          >
            <button
              type="button"
              onClick={onToggleDock}
              aria-label="侧栏"
              aria-pressed={dockOpen}
              data-dock-state={dockOpen ? "open" : "closed"}
              data-dock-tab={dockTab ?? undefined}
              className={cn(
                "group flex h-8 items-center gap-0 rounded-full border px-2 text-[12px] transition-all duration-200 ease-out active:scale-95 cursor-pointer",
                "border-[color-mix(in_srgb,var(--husk-n200)_90%,transparent)] shadow-[0_2px_8px_rgba(0,0,0,0.08)] dark:shadow-[0_2px_8px_rgba(0,0,0,0.35)]",
                // One look whether the sidebar is open or closed — an open
                // card is not a selected tab, and the grey "active" fill read
                // as exactly that. Hover is the only state change.
                "bg-white text-neutral-600 hover:bg-neutral-50 hover:text-neutral-900 dark:bg-[var(--husk-active)] dark:text-neutral-300 dark:hover:bg-[var(--husk-hover)]",
                // The leader carries the row's spacing: the follower is a
                // 32px icon, and the gap belongs between them, not around it.
                jobsPillVisible && "mr-2",
              )}
            >
              {/* The sidebar's state, not just a badge: the glyph follows
                  what the pill would show — a diff marker while 改动 is up, a
                  box while 产物 is, and an “open the sidebar” arrow when it is
                  closed. */}
              <DockGlyph open={!!dockOpen} tab={dockTab} />
              {/* The numbers are the view's own: a diff reads as its line
                  counts (+/−), a document list as its file count. */}
              <span className={pillTextClass(false)}>
                {dockTab === "changes" ? (
                  <>
                    <span className="flex-none font-mono tabular-nums whitespace-nowrap text-emerald-600 dark:text-emerald-400">
                      +{dockAdds}
                    </span>
                    <span className="flex-none font-mono tabular-nums whitespace-nowrap text-rose-600 dark:text-rose-400">
                      −{dockDels}
                    </span>
                  </>
                ) : (
                  <span className="flex-none whitespace-nowrap font-mono tabular-nums opacity-70">
                    {dockCount} 个文件
                  </span>
                )}
              </span>
            </button>
          </TooltipSimple>
        )}
        {jobsPillVisible && jobs && (
          <TooltipSimple content={jobsOpen ? "关闭后台任务" : `后台任务 · ${jobs.text}`} side="top">
            <button
              type="button"
              onClick={onToggleJobs}
              aria-label="后台任务"
              aria-pressed={jobsOpen}
              data-jobs-tone={jobs.tone}
              data-compact={jobsCollapsed ? "true" : undefined}
              className={cn(
                "group flex h-8 items-center gap-0 rounded-full border px-2 text-[12px] transition-all duration-200 ease-out active:scale-95 cursor-pointer",
                "border-[color-mix(in_srgb,var(--husk-n200)_90%,transparent)] shadow-[0_2px_8px_rgba(0,0,0,0.08)] dark:shadow-[0_2px_8px_rgba(0,0,0,0.35)]",
                "bg-white text-neutral-600 hover:bg-neutral-50 hover:text-neutral-900 dark:bg-[var(--husk-active)] dark:text-neutral-300 dark:hover:bg-[var(--husk-hover)]",
              )}
            >
              {/* One 16px box for every tone: the pill's collapsed width is
                  the icon's, and it must not jump 2px when a job fails. */}
              <span className="flex h-4 w-4 flex-none items-center justify-center">
                {jobs.tone === "running" ? (
                  // The orb's own travel, not a spinner: this pill sits on the
                  // same row as the session's activity glyphs, so it reads as
                  // the same family of motion.
                  <Orb variant="B2" size={16} className="text-sky-600 dark:text-sky-400" />
                ) : jobs.tone === "failed" ? (
                  <CircleX className="h-4 w-4 text-rose-600 dark:text-rose-400" />
                ) : (
                  // A touch smaller than the state glyphs beside it: a filled
                  // check carries more ink than an outlined cross at one size.
                  <CircleCheck className="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400" />
                )}
              </span>
              <span className={pillTextClass(jobsCollapsed)}>
                <span
                  className={cn(
                    "flex-none whitespace-nowrap",
                    jobs.tone === "failed" && "text-rose-600 dark:text-rose-400",
                  )}
                >
                  {jobs.text}
                </span>
              </span>
            </button>
          </TooltipSimple>
        )}
        {subagentPillVisible && subagentPill && (
          <Popover>
            <PopoverTrigger asChild>
              <button
                type="button"
                aria-label="子代理"
                data-compact={subagentCollapsed ? "true" : undefined}
                className={cn(
                  "group flex h-8 items-center gap-0 rounded-full border px-2 text-[12px] transition-all duration-200 ease-out active:scale-95 cursor-pointer",
                  "border-[color-mix(in_srgb,var(--husk-n200)_90%,transparent)] shadow-[0_2px_8px_rgba(0,0,0,0.08)] dark:shadow-[0_2px_8px_rgba(0,0,0,0.35)]",
                  "bg-white text-neutral-600 hover:bg-neutral-50 hover:text-neutral-900 dark:bg-[var(--husk-active)] dark:text-neutral-300 dark:hover:bg-[var(--husk-hover)]",
                )}
              >
                <span className="flex h-4 w-4 flex-none items-center justify-center">
                  <Orb variant="G2" size={12} label="子代理" className="text-violet-600 dark:text-violet-400" />
                </span>
                <span className={pillTextClass(subagentCollapsed)}>
                  <span className="flex-none whitespace-nowrap">{subagentPill.text}</span>
                </span>
              </button>
            </PopoverTrigger>
            {/* The children have no rows anywhere else — this is where what
                each of them was handed stays readable while it runs. */}
            <PopoverContent side="top" align="center" className="w-80 p-1.5">
              <div className="flex flex-col gap-0.5">
                {(subagents ?? []).map((c) => (
                  <div key={c.childId} className="flex flex-col gap-1 rounded-md px-2 py-1.5">
                    <div className="flex items-start gap-2">
                      <Orb variant="G2" size={12} className="mt-0.5 flex-none text-violet-600 dark:text-violet-400" />
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-[12px]">{c.name}</div>
                        <div className="truncate text-[11px] text-neutral-500 dark:text-neutral-400">{c.task}</div>
                      </div>
                    </div>
                    {/* A parked child takes the text only after it resumes —
                        saying so beats a steer that looks swallowed. */}
                    {c.waiting != null && (
                      <div className="pl-5 text-[11px] leading-relaxed text-amber-600 dark:text-amber-400">
                        {WAITING_STEER_HINT}
                      </div>
                    )}
                    <div className="pl-5">
                      <ChildSteerInput childId={c.childId} />
                    </div>
                  </div>
                ))}
              </div>
            </PopoverContent>
          </Popover>
        )}
        <div
          className={cn(
            // No `overflow-hidden`: it would clip the round button's shadow.
            // The button fades out as the box collapses, so it never shows
            // outside the reserved space.
            "flex-none transition-all duration-200 ease-out",
            anyPill
              ? // Beside the pill: the slot grows from zero, the pill yields
                // to the right edge it just opened up.
                !isAtBottom
                ? "ml-2 w-8 opacity-100"
                : "pointer-events-none ml-0 w-0 opacity-0"
              : // Alone: the button keeps its width so the row's centre never
                // moves — it only fades, rising from the composer's edge.
                cn(
                  "ml-0 w-8",
                  isAtBottom
                    ? "pointer-events-none translate-y-2 opacity-0"
                    : "translate-y-0 opacity-100",
                ),
          )}
        >
          <TooltipSimple content="回到底部" side="top">
            <button
              type="button"
              onClick={scrollToBottom}
              aria-label="回到底部"
              className="w-8 h-8 rounded-full bg-white dark:bg-[var(--husk-active)] hover:bg-neutral-50 dark:hover:bg-[var(--husk-hover)] text-neutral-600 hover:text-neutral-900 flex items-center justify-center border border-[color-mix(in_srgb,var(--husk-n200)_90%,transparent)] shadow-[0_2px_8px_rgba(0,0,0,0.08)] dark:shadow-[0_2px_8px_rgba(0,0,0,0.35)] transition-all active:scale-95 cursor-pointer"
            >
              <ArrowDown className="h-4 w-4" />
            </button>
          </TooltipSimple>
        </div>
      </div>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent className="min-w-[160px]">
        <ContextMenuItem
          className="gap-2 text-xs cursor-pointer"
          disabled={!hasSelection}
          onClick={() => void copySelection()}
        >
          <Copy className="h-3.5 w-3.5" />
          复制
        </ContextMenuItem>
        <ContextMenuItem
          className="gap-2 text-xs cursor-pointer"
          disabled={!hasSelection}
          onClick={askAboutSelection}
        >
          <TextQuote className="h-3.5 w-3.5" />
          就选中内容提问
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem
          className="gap-2 text-xs cursor-pointer"
          onClick={() => void pasteToComposer()}
        >
          <FileArrowRight className="h-3.5 w-3.5" />
          粘贴到输入框
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}
