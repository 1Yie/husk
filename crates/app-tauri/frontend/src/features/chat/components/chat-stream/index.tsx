
import { memo, useEffect, useRef, useState, useCallback, useMemo, startTransition, type ReactNode } from "react";
import { MemoStreamdown } from "@/features/chat/components/chat-stream/markdown-stream";
import type { SessionView, StreamItem } from "@/features/chat/hooks/stream-view";
import { AssistantStatus } from "@/features/chat/components/assistant-status/index";
import { ChatSkeleton, OlderPageSkeleton } from "@/features/chat/components/chat-skeleton/index";
import { ToolChips, type ToolChipRow } from "@/features/chat/components/tool-chips/index";
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
} from "@keyline-icons/react";
import { chatMarkdownComponents } from "@/features/chat/components/chat-stream/markdown-components";
import { TooltipSimple } from "@/components/ui/tooltip";
import { ChatTurnRail, type RailMark } from "@/features/chat/components/chat-stream/chat-turn-rail";
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
import { loadStamp } from "@/lib/load-probe";
import { timeGreeting } from "@/lib/greeting";
import { readAttachment } from "@/lib/agent-ipc/sessions";
import { retryTurn } from "@/lib/agent-ipc/commands";

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
      return step.text.trim().replace(/\s+/g, " ");
    }
  }
  for (const step of steps) {
    if (step.type === "thinking" && step.text.trim()) {
      return `思考: ${step.text.trim().replace(/\s+/g, " ")}`;
    }
  }
  for (const step of steps) {
    if (step.type === "tools" && step.rows.length > 0) {
      return `工具: ${step.rows.map((r) => r.label).join(", ")}`;
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
  /** Fetch + prepend the next older page (scroll-top trigger). */
  /** Fetch + prepend the next older page. Resolves `true` when a
   * page actually landed — `false` (empty/end) clears the pending
   * scroll-restore so a stale geometry can't be consumed later. */
  onLoadOlder?: () => Promise<boolean>;
}

type AssistantStep =
  | { type: "thinking"; text: string; done: boolean }
  | { type: "text"; text: string; streaming: boolean }
  | { type: "tools"; rows: ToolChipRow[] }
  | { type: "system"; text: string };

interface Turn {
  id: string;
  /** Global history index of the turn's first item — lets a placeholder
   * click-seek target the right turn after its page loads. */
  hi?: number;
  userText?: string;
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
  align,
}: {
  ts: number;
  copyText?: string;
  onRetry?: () => void;
  align: "start" | "end";
}) {
  const [copied, setCopied] = useState(false);
  const [retryOpen, setRetryOpen] = useState(false);
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
      <span
        className="cursor-default px-0.5 tabular-nums"
        data-tooltip={formatFullTime(ts)}
      >
        {formatTurnTime(ts)}
      </span>
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

  const flush = (end: number) => {
    if (currentTurn) {
      out.push({ turn: currentTurn, span: items.slice(turnStart, end) });
      currentTurn = null;
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
        ts: item.ts,
        steps: [],
      };
      turnStart = idx;
      continue;
    }

    const t = ensureTurn(item, idx);
    const lastStep = t.steps[t.steps.length - 1];

    if (item.kind === "thinking") {
      t.steps.push({
        type: "thinking",
        text: item.text,
        done: item.done,
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

    if (item.kind === "tool" || item.kind === "approval") {
      // Batch children — attach to the nearest preceding batch_execute
      // row in the current step instead of counting as a top-level call.
      if (item.kind === "tool" && item.parent) {
        const lastStep = t.steps[t.steps.length - 1];
        if (lastStep?.type === "tools") {
          // Parent may be a top-level row (batch) or a nested agent card
          // (a subagent's own tool calls) — search both, newest first.
          const findParent = (rows: ToolChipRow[]): ToolChipRow | null => {
            for (let j = rows.length - 1; j >= 0; j--) {
              if (rows[j].label === item.parent) return rows[j];
              const nested = rows[j].children ? findParent(rows[j].children!) : null;
              if (nested) return nested;
            }
            return null;
          };
          const parentRow = findParent(lastStep.rows);
          if (parentRow) {
              const child: ToolChipRow = {
                id: `tool-${idx}`,
                // A subagent is a second agent, not a tool call: give it its
                // own card so it does not read as one more line in the list.
                kind: item.parent === "delegate" ? "agent" : undefined,
                label: item.name,
                chip:
                  item.parent === "delegate"
                    ? (item.args ?? "")
                    : item.args && item.args !== item.name
                      ? item.args
                      : "",
                status:
                  item.content === undefined
                    ? "running"
                    : item.ok === false
                      ? "aborted"
                      : "done",
                // Finished → the report; still running → whatever it has
                // streamed so far (reasoning first, then the answer).
                detail: item.content
                  ? [item.content]
                  : item.live || item.liveReasoning
                    ? [[item.liveReasoning, item.live].filter(Boolean).join("\n\n")]
                    : undefined,
                uiType: item.uiType,
              };
              parentRow.children = [...(parentRow.children ?? []), child];
              continue;
          }
          continue;
        }
      }
      const row: ToolChipRow =
        item.kind === "tool"
          ? {
              id: `tool-${idx}`,
              label: item.name,
              chip: item.args && item.args !== item.name ? item.args : "",
              status:
                item.content === undefined ? "running" : item.ok === false ? "aborted" : "done",
              detail: item.content
                ? [item.content]
                : item.approval?.diff
                  ? [item.approval.diff]
                  : undefined,
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

      if (lastStep?.type === "tools") {
        lastStep.rows.push(row);
      } else {
        t.steps.push({
          type: "tools",
          rows: [row],
        });
      }
    }
  }

  flush(end);
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
  system: "提示",
};

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

/** One turn's full row — user bubble, assistant steps, turn-end footer.
 * `memo` + the incremental parse cache mean a settled turn's `turn`
 * object survives unchanged across flushes, so a streaming delta only
 * re-renders the LAST turn instead of reconciling the whole history.
 * `streaming` is `view.streaming && isLast` — folded in by the caller so
 * a non-last turn's props stay stable while a turn runs. */
const ChatTurn = memo(function ChatTurn({
  turn,
  isLast,
  streaming,
}: {
  turn: Turn;
  isLast: boolean;
  streaming: boolean;
}) {
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
      className="flex w-full flex-col gap-6"
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
          <div className="bg-neutral-100 text-neutral-900 flex w-fit flex-col gap-2 rounded-xl px-3.5 py-2.5 text-[14px]">
            <div className="min-w-0 text-[14px] leading-relaxed [&_p]:max-w-none [&_p:first-child]:mt-0 [&_p:last-child]:mb-0 select-text [overflow-wrap:anywhere]">
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
          className="flex w-full flex-col items-start gap-4"
        >
          {turn.steps.map((step, stepIdx) => {
            if (step.type === "thinking") {
              const thinkingText = step.text.trim();
              if (!step.done && !thinkingText) return null;
              return (
                <div
                  key={`step-${stepIdx}`}
                  id={`chat-turn-${turn.id}-step-${stepIdx}`}
                  className="w-full"
                >
                  <AssistantStatus
                    mode={step.done ? "thought" : "thinking"}
                    thinkingText={thinkingText}
                  />
                </div>
              );
            }

            if (step.type === "text") {
              const isLastStep = stepIdx === turn.steps.length - 1;
              const isAnimating = streaming && isLastStep;

              return (
                <div
                  id={`chat-turn-${turn.id}-step-${stepIdx}`}
                  className="w-full min-w-0 text-[14px] text-neutral-900 [&_p]:max-w-none"
                  key={`step-${stepIdx}`}
                >
                  <CappedAssistantText text={step.text} animating={isAnimating} />
                </div>
              );
            }

            if (step.type === "tools") {
              const running = step.rows.some((r) => r.status === "running");
              return (
                <div
                  id={`chat-turn-${turn.id}-step-${stepIdx}`}
                  className="flex w-full min-w-0 flex-col items-start gap-2"
                  key={`step-${stepIdx}`}
                >
                  {running && <AssistantStatus mode="tools" thinkingText="" />}
                  <ToolChips
                    rows={step.rows}
                    renderProse={(text) => (
                      <MemoStreamdown text={text} animating={running} />
                    )}
                  />
                </div>
              );
            }

            if (step.type === "system") {
              return (
                <div
                  key={`step-${stepIdx}`}
                  id={`chat-turn-${turn.id}-step-${stepIdx}`}
                  className="text-xs text-neutral-500 font-mono py-1 select-none"
                >
                  {step.text}
                </div>
              );
            }

            return null;
          })}
        </div>
      )}
      {/* ts falls back to the prompt's for text-less turns;
        * hidden while this turn streams. */}
      {turn.steps.length > 0 &&
        !streaming &&
        (turn.assistantTs ?? turn.ts) != null && (
          <div className="-mt-4">
            <TurnFooter
              align="start"
              ts={(turn.assistantTs ?? turn.ts) as number}
              copyText={copyText}
              onRetry={isLast ? () => void retryTurn() : undefined}
            />
          </div>
        )}
    </div>
  );
});

export function ChatStream({ view, bottomPad = 128, composerH, loading, sessionKey, hasMore, onLoadOlder }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const endRef = useRef<HTMLDivElement>(null);
  const pinnedRef = useRef(true);
  const [isAtBottom, setIsAtBottom] = useState(true);
  // Index of the last user item seen — a newly appended one means the
  // user just submitted, which always re-pins to the bottom.
  const lastUserIdxRef = useRef<unknown>(null);

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
  useEffect(() => {
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
    foldTurnSpans(items, cursor, items.length, spans);
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
  // Leading turns kept unmounted — decremented by backfill ticks to 0.
  // `hiddenTop` only ever SHRINKS: the old tail-anchored `mounted` count
  // let a newly appended turn push the oldest mounted turn out of the
  // slice — it unmounted (big height loss above the viewport) then
  // remounted one tick later (height back), which is exactly the
  // jump-to-bottom-then-fling-upward the mount window was causing.
  const [hiddenTop, setHiddenTop] = useState<number | null>(null);
  const [mountedKey, setMountedKey] = useState(sessionKey);
  if (sessionKey !== mountedKey) {
    setMountedKey(sessionKey);
    setHiddenTop(null);
  }
  const lastStartRef = useRef(view.historyStart);
  if (view.historyStart !== lastStartRef.current) {
    lastStartRef.current = view.historyStart;
    // Prepended page mounts in full — the scroll-restore delta must
    // measure the real added height.
    setHiddenTop(0);
  }
  if (hiddenTop === null && turns.length > 0) {
    setHiddenTop(Math.max(0, turns.length - MOUNT_CHUNK));
  }
  const hidden = hiddenTop ?? Math.max(0, turns.length - MOUNT_CHUNK);
  useEffect(() => {
    if (!loading) loadStamp("content-mounted");
  }, [loading]);

  useEffect(() => {
    if (hidden <= 0) return;
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
        setHiddenTop((h) => Math.max(0, (h ?? hidden) - BACKFILL_CHUNK)),
      ),
    );
    if (hidden - BACKFILL_CHUNK <= 0) loadStamp("backfill-done");
    return () => cancel(id as number);
  }, [hidden]);

  const visibleTurns = hidden > 0 ? turns.slice(hidden) : turns;


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
    const loadedUserTurns = turns.reduce((n, t) => n + (t.userText ? 1 : 0), 0);
    // Exact when the backend reported `turn_offset` — the ordinal of the
    // first turn inside the loaded slice IS the unloaded count. The
    // count-based figure stays as the fallback for views built before
    // that field existed, or for live-only ones.
    const unloaded = Math.max(
      0,
      view.turnOffset ?? (view.turnTotal ?? loadedUserTurns) - loadedUserTurns,
    );
    // The page is a slice of MESSAGES, so its first turn is usually a HALF
    // turn: the cut landed mid-answer and `turnOffset` still counts that
    // turn as wholly unloaded, while its answer already sits on screen as
    // the first loaded mark. Crediting it two dots invented a whole extra
    // turn in the unloaded share and dropped the load boundary one row too
    // far — the rail then disagreed with the content about where the
    // unloaded region ends. Only its prompt is missing, so it gets exactly
    // ONE trailing dot, and that dot points at the answer it belongs to.
    const boundaryTurn = turns[0] && !turns[0].userText ? turns[0] : null;
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
    // was not looking at.
    const firstTurnNum = unloaded + (boundaryTurn ? 0 : 1);
    turns.forEach((turn, idx) => {
      const turnNum = firstTurnNum + idx;
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
          previewTitle: `用户提问 #${turnNum}`,
          previewSnippet: clipPreview(userPreview),
          len: userPreview.length,
        });
      }

      if (turn.steps.length > 0) {
        const isLastTurn = idx === turns.length - 1;
        // One mark per step, not per turn: a turn is the prompt plus every block
        // the answer arrived in (thinking / tools / prose), and a single dash per
        // turn flattened agentic turns into stubs.
        turn.steps.forEach((step, stepIdx) => {
          const isStreaming =
            view.streaming && isLastTurn && stepIdx === turn.steps.length - 1;
          list.push({
            id: `mark-${turn.id}-step-${stepIdx}`,
            type: "assistant",
            turnId: turn.id,
            targetId: `chat-turn-${turn.id}-step-${stepIdx}`,
            previewTitle: `助手回复 #${turnNum} · ${STEP_LABEL[step.type]}${
              isStreaming ? " (生成中)" : ""
            }`,
            previewSnippet: stepPreview(step),
            len: stepWeight(step),
            isStreaming,
          });
        });
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
  }, [turns, view.streaming, showReplyWait, view.turnOffset, view.turnTotal]);

  // Windowed shells change scrollHeight whenever they mount, unmount,
  // or measure real heights. While pinned, follow the bottom; while not
  // pinned, re-measure mark positions (debounced — a scroll pass mounts
  // rows continuously and each resize would otherwise re-measure mid-
  // gesture).
  const measureMarks = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const containerTop = el.getBoundingClientRect().top;
    const base = el.scrollTop - containerTop;
    const tops: { id: string; top: number }[] = [];
    let prev = -Infinity;
    for (const mark of marks) {
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
  }, [marks]);

  const appliedStartRef = useRef(view.historyStart);
  useEffect(() => {
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
  }, [marks]);

  // Re-measure after every layout change (items, window slice) AND
  // re-derive the active mark from those fresh positions: measuring alone
  // left the rail pointing at a stale mark (its initial "mark-top") until
  // the user happened to scroll.
  useEffect(() => {
    measureMarks();
    updateActiveMark();
    // `loading` matters: the reveal commit is what first renders real turns,
    // and the effect above it ran against the skeleton (nothing to measure).
  }, [measureMarks, updateActiveMark, view.items, hidden, loading]);

  useEffect(() => {
    const el = scrollRef.current;
    const col = el?.firstElementChild;
    if (!el || !col) return;
    let debounce = 0;
    const ro = new ResizeObserver(() => {
      // Growth below the viewport → re-anchor the bottom while following.
      // The pin's own scroll events can't clear follow-mode anymore (only
      // a real gesture can), so the landing converges frame by frame.
      if (pinnedRef.current) {
        el.scrollTo({ top: el.scrollHeight, behavior: "instant" });
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
    return () => {
      window.clearTimeout(debounce);
      ro.disconnect();
    };
  }, [measureMarks, updateActiveMark]);

  useEffect(() => {
    // Own-message jump: a new `kind:"user"` item means the user just
    // submitted — snap to the bottom even if they'd scrolled up to read
    // (sending a message with no visible response feels broken).
    // Identity check, NOT index: a prepended page shifts the last user
    // item's index by the page size and would otherwise read as a fresh
    // send — yanking the viewport to the bottom mid-read.
    let lastUser: (typeof view.items)[number] | null = null;
    for (let i = view.items.length - 1; i >= 0; i--) {
      if (view.items[i].kind === "user") {
        lastUser = view.items[i];
        break;
      }
    }
    const lastUserKey = lastUser ? (lastUser.hi ?? lastUser) : null;
    if (lastUserKey !== null && lastUserKey !== lastUserIdxRef.current) {
      pinnedRef.current = true;
    }
    lastUserIdxRef.current = lastUserKey;
    if (pinnedRef.current) {
      endRef.current?.scrollIntoView({ block: "end" });
      setIsAtBottom(true);
      if (marks.length > 0) {
        activeIdxRef.current = marks.length - 1;
        setActiveMarkId(marks[marks.length - 1].id);
      }
    } else {
      const el = scrollRef.current;
      if (el) {
        setIsAtBottom(el.scrollHeight - el.scrollTop - el.clientHeight < 48);
      }
    }
  }, [view.items, marks]);

  const markRafRef = useRef(0);
  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    if (el.scrollTop < 480) void maybeLoadOlder();
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
      const loadedUser = turnsRef.current.reduce((n, t) => n + (t.userText ? 1 : 0), 0);
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
          const userTurns = loaded.filter((t) => t.userText);
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
      isSmoothScrollingRef.current = true;
      el.scrollIntoView({ behavior: "smooth", block: "start" });
      setTimeout(() => {
        isSmoothScrollingRef.current = false;
      }, 500);
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
      >
        <div className="max-w-3xl w-full mx-auto px-4 pt-6 flex flex-col gap-6 min-h-full">
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

              {visibleTurns.map((turn, turnIdx) => {
                const isLast = turnIdx === visibleTurns.length - 1;
                return (
                  <ChatTurn
                    key={turn.id}
                    turn={turn}
                    isLast={isLast}
                    streaming={view.streaming && isLast}
                  />
                );
              })}

            {showReplyWait && (
              <div id="chat-reply-wait">
                <AssistantStatus mode="reply" thinkingText="" />
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

      <div
        style={{ bottom: `${(composerH ?? (bottomPad ? bottomPad - 24 : 140)) + 12}px` }}
        className={cn(
          "absolute left-1/2 -translate-x-1/2 z-30 transition-all duration-200 ease-out",
          !isAtBottom
            ? "opacity-100 translate-y-0 pointer-events-auto"
            : "opacity-0 translate-y-2 pointer-events-none"
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
