
import { memo, useEffect, useRef, useState, useCallback, useMemo, type ReactNode } from "react";
import { cjk } from "@streamdown/cjk";
import { Streamdown } from "streamdown";
import type { SessionView, StreamItem } from "../../hooks/stream-view";
import { AssistantStatus } from "../assistant-status";
import { ChatSkeleton, OlderPageSkeleton } from "../chat-skeleton";
import { ToolChips, type ToolChipRow } from "../tool-chips";
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
import { chatMarkdownComponents } from "./markdown-components";
import { TooltipSimple } from "@/components/ui/tooltip";
import { prismCodePlugin, prismCodePluginStreaming } from "../../lib/syntax-highlight";
import { ChatTurnRail, type RailMark } from "./chat-turn-rail";
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
import { requestQuote, selectionWithin } from "../../lib/selection-bus";
import { cn } from "@/lib/utils";
import { loadStamp } from "@/lib/load-probe";
import { timeGreeting } from "@/lib/greeting";
import { readAttachment } from "../../invoke/agent/sessions";
import { retryTurn } from "../../invoke/agent/commands";

const streamdownIcons = {
  CheckIcon: Check,
  CopyIcon: Copy,
};

/** Streamdown in `memo`: a finished turn's `text` reference is stable, so only the
 * block whose text changed re-renders. `plugins` is memoized on `animating` — a
 * fresh object would defeat Streamdown's own per-block memo (its comparator includes
 * `plugins`) and re-render every code block; while streaming it also selects the
 * complete-lines-only highlighter. */
const MemoStreamdown = memo(function MemoStreamdown({
  text,
  animating,
}: {
  text: string;
  animating?: boolean;
}) {
  const plugins = useMemo(
    () => ({ cjk, code: (animating ? prismCodePluginStreaming : prismCodePlugin) as any }),
    [animating],
  );
  return (
    <Streamdown
      isAnimating={animating}
      plugins={plugins}
      shikiTheme={["github-dark", "github-dark"]}
      components={chatMarkdownComponents}
      icons={streamdownIcons}
    >
      {text}
    </Streamdown>
  );
});

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
  return (
    <Streamdown
      isAnimating={false}
      plugins={{ cjk, code: prismCodePlugin as any }}
      shikiTheme={["github-dark", "github-dark"]}
      components={userMarkdownComponents}
      icons={streamdownIcons}
    >
      {text}
    </Streamdown>
  );
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

function parseTurns(items: StreamItem[]): Turn[] {
  const turns: Turn[] = [];
  let currentTurn: Turn | null = null;

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
      turns.push(currentTurn);
    }
    return currentTurn;
  };

  for (let idx = 0; idx < items.length; idx++) {
    const item = items[idx];

    if (item.kind === "user") {
      currentTurn = {
        id: turnId(item, idx),
        hi: item.hi,
        userText: item.text,
        ts: item.ts,
        steps: [],
      };
      turns.push(currentTurn);
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

  return turns;
}

/** Turns mount all at once, behind the loading veil; `content-visibility: auto` then
 *  skips offscreen layout and paint, so scrolling costs nothing per turn. */

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
      <MemoStreamdown text={shown} animating={animating} />
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

export function ChatStream({ view, bottomPad = 128, composerH, loading, sessionKey, hasMore, onLoadOlder }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const endRef = useRef<HTMLDivElement>(null);
  const pinnedRef = useRef(true);
  const [isAtBottom, setIsAtBottom] = useState(true);
  // Index of the last user item seen — a newly appended one means the
  // user just submitted, which always re-pins to the bottom.
  const lastUserIdxRef = useRef<unknown>(null);

  useEffect(() => {
    // Session switch must land at the newest message — the scroll
    // container is reused across sessions, so its scrollTop and the
    // pinned flag would otherwise keep whatever position the previous
    // session left (that's why some switches opened at the oldest turn).
    pinnedRef.current = true;
    setIsAtBottom(true);
    lastUserIdxRef.current = null;
    requestAnimationFrame(() => {
      const el = scrollRef.current;
      if (el) el.scrollTo({ top: el.scrollHeight, behavior: "instant" });
    });
  }, [sessionKey]);

  // Land at the newest message once content actually mounts. The
  // sessionKey rAF above fires against the empty/skeleton container and
  // the [view.items] effect can't reach endRef while `loading` still
  // shows the skeleton — so neither reliably lands the scroll. This
  // fires the moment loading clears with items present, then re-snaps
  // across the next frames while shells settle into real heights (the
  // column RO keeps re-pinning afterwards).
  const [landed, setLanded] = useState(false);
  useEffect(() => {
    if (landed || loading || view.items.length === 0) return;
    setLanded(true);
    pinnedRef.current = true;
    setIsAtBottom(true);
    // One snap — the mounted tail is already measured height, and the
    // column RO re-pins on every backfill growth anyway. Extra rAF snaps
    // just force full layouts of the fresh tree for nothing.
    const el = scrollRef.current;
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: "instant" });
  }, [landed, loading, view.items]);

  // ---------- older-history pagination ----------
  // Scroll-near-top fetches the next older page; the prepend grows the
  // column above the viewport, so `restoreRef` carries pre-fetch
  // geometry and the items effect adds back the delta — viewport stays
  // glued to the same message instead of jumping.
  const fetchingRef = useRef(false);
  const [fetching, setFetching] = useState(false);
  const restoreRef = useRef<{ top: number; height: number } | null>(null);

  const maybeLoadOlder = useCallback(async () => {
    const el = scrollRef.current;
    if (!el || !onLoadOlder || !hasMore || fetchingRef.current) return;
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
  }, [hasMore, onLoadOlder]);

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
      setActiveMarkId(marks[marks.length - 1].id);
    }
  };

  const turns = useMemo(() => parseTurns(view.items), [view.items]);

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
    const t = setTimeout(
      () => setHiddenTop((h) => Math.max(0, (h ?? hidden) - BACKFILL_CHUNK)),
      0,
    );
    if (hidden - BACKFILL_CHUNK <= 0) loadStamp("backfill-done");
    return () => clearTimeout(t);
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
        previewSnippet: "会话起点",
      },
    ];

    // Unloaded-history placeholders — the rail is an overview of the
    // WHOLE session, so turns above the loaded page still occupy their
    // share as faded dashes (click pages up to that region).
    const loadedUserTurns = turns.reduce((n, t) => n + (t.userText ? 1 : 0), 0);
    const unloaded = Math.max(0, (view.turnTotal ?? loadedUserTurns) - loadedUserTurns);
    const PH_CAP = 120;
    const phCount = Math.min(unloaded, PH_CAP);
    for (let i = 0; i < phCount; i++) {
      // Even mapping back onto the real ordinal range when capped.
      const phIndex = Math.round((i / Math.max(1, phCount - 1)) * Math.max(0, unloaded - 1));
      list.push({
        id: `mark-ph-${i}`,
        type: "placeholder",
        phIndex,
        targetId: "",
        previewTitle: "",
        previewSnippet: "",
      });
    }

    turns.forEach((turn, idx) => {
      const turnNum = idx + 1;
      if (turn.userText) {
        list.push({
          id: `mark-${turn.id}-user`,
          type: "user",
          turnId: turn.id,
          targetId: `chat-turn-${turn.id}-user`,
          previewTitle: `用户提问 #${turnNum}`,
          previewSnippet: getUserPreview(turn.userText),
        });
      }

      if (turn.steps.length > 0) {
        const isLastTurn = idx === turns.length - 1;
        const isStreaming = view.streaming && isLastTurn;
        list.push({
          id: `mark-${turn.id}-assistant`,
          type: "assistant",
          turnId: turn.id,
          targetId: `chat-turn-${turn.id}-assistant`,
          previewTitle: `助手回复 #${turnNum}${isStreaming ? " (生成中)" : ""}`,
          previewSnippet: getAssistantPreview(turn.steps),
          isStreaming,
        });
      } else if (idx === turns.length - 1 && (view.streaming || showReplyWait)) {
        list.push({
          id: `mark-${turn.id}-assistant-waiting`,
          type: "assistant",
          turnId: turn.id,
          targetId: "chat-reply-wait",
          previewTitle: `助手回复 #${turnNum} (等待回复)`,
          previewSnippet: "正在等待回复...",
          isStreaming: true,
        });
      }
    });

    return list;
  }, [turns, view.streaming, showReplyWait]);

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
    for (const mark of marks) {
      if (mark.type === "top") continue;
      const target = document.getElementById(mark.targetId);
      if (target) {
        tops.push({ id: mark.id, top: target.getBoundingClientRect().top + base });
      }
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
    if (!r || !el) return;
    el.scrollTop = r.top + (el.scrollHeight - r.height);
    measureMarks();
  }, [view.historyStart, view.items, measureMarks]);

  useEffect(() => {
    const el = scrollRef.current;
    const col = el?.firstElementChild;
    if (!el || !col) return;
    let debounce = 0;
    const ro = new ResizeObserver(() => {
      if (pinnedRef.current) {
        el.scrollTo({ top: el.scrollHeight, behavior: "instant" });
      }
      window.clearTimeout(debounce);
      debounce = window.setTimeout(measureMarks, 250);
    });
    ro.observe(col);
    return () => {
      window.clearTimeout(debounce);
      ro.disconnect();
    };
  }, [measureMarks]);


  const [activeMarkId, setActiveMarkId] = useState<string>("mark-top");

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
  // Offset of each marked block inside its turn shell — refreshed while
  useEffect(() => {
    measureMarks();
  }, [measureMarks, view.items, hidden]);

  const updateActiveMark = useCallback(() => {
    if (isSmoothScrollingRef.current) return;
    const el = scrollRef.current;
    if (!el || marks.length === 0) return;

    const { scrollTop, scrollHeight, clientHeight } = el;

    if (scrollTop < 32) {
      setActiveMarkId(marks[0].id);
      return;
    }

    if (scrollHeight - scrollTop - clientHeight < 48) {
      setActiveMarkId(marks[marks.length - 1].id);
      return;
    }

    // Binary search the last mark above the trigger line — cached
    // positions, zero layout reads.
    const trigger = scrollTop + 100;
    const tops = markTopsRef.current;
    let lo = 0, hi = tops.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (tops[mid].top <= trigger) lo = mid + 1;
      else hi = mid;
    }
    const currentId = lo > 0 ? tops[lo - 1].id : marks[0].id;
    setActiveMarkId(currentId);
  }, [marks]);

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
      pinnedRef.current = atBottom;
    }
    setIsAtBottom(atBottom);
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
      setActiveMarkId(mark.id);
      isSmoothScrollingRef.current = true;
      scrollRef.current?.scrollTo({ top: 0, behavior: "smooth" });
      setTimeout(() => {
        isSmoothScrollingRef.current = false;
      }, 500);
      return;
    }

    if (mark.type === "placeholder") {
      // Page up until the estimated history index is loaded, then scroll
      // to the turn that covers it. `viewRef` tracks the freshest view
      // because `onLoadOlder` resolves before the prop does.
      const loadedUser = turnsRef.current.reduce((n, t) => n + (t.userText ? 1 : 0), 0);
      const unloadedNow = Math.max(0, (viewRef.current.turnTotal ?? loadedUser) - loadedUser);
      const start = viewRef.current.historyStart ?? 0;
      const approx = Math.round(((mark.phIndex ?? 0) + 0.5) * (start / Math.max(1, unloadedNow)));
      void (async () => {
        let guard = 0;
        while (guard++ < 12 && (viewRef.current.historyStart ?? 0) > approx) {
          // maybeLoadOlder carries the fetching guard + scroll-restore
          // bookkeeping — calling onLoadOlder directly could double-fetch.
          await maybeLoadOlder();
          await new Promise((r) => setTimeout(r, 0));
        }
        const loaded = turnsRef.current;
        const target = loaded.find((t) => t.hi != null && t.hi >= approx)
          ?? loaded[loaded.length - 1];
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
    setActiveMarkId(mark.id);

    if (isLast) {
      isSmoothScrollingRef.current = true;
      endRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
      setTimeout(() => {
        isSmoothScrollingRef.current = false;
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

  const handleDragScroll = (ratio: number) => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = ratio >= 0.98;
    pinnedRef.current = atBottom;
    setIsAtBottom(atBottom);
    const maxScroll = el.scrollHeight - el.clientHeight;
    if (maxScroll > 0) {
      el.scrollTop = ratio * maxScroll;
      updateActiveMark();
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
        onDragScroll={handleDragScroll}
        loading={loading}
      />

      <div className="stream-scroll" ref={scrollRef} onScroll={onScroll}>
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

              {visibleTurns.map((turn, turnIdx) => (
                <div
                  key={turn.id}
                  id={`chat-turn-${turn.id}`}
                  className="flex w-full flex-col gap-6"
                >
                  {turn.userText && (
                    <div
                      id={`chat-turn-${turn.id}-user`}
                      className="ms-auto flex w-fit max-w-[80%] flex-col items-end gap-1"
                    >
                      <div className="bg-neutral-100 text-neutral-900 flex w-fit flex-col gap-2 rounded-xl px-3.5 py-2.5 text-[14px]">
                        <div className="min-w-0 text-[14px] leading-relaxed [&_p]:max-w-none [&_p:first-child]:mt-0 [&_p:last-child]:mb-0 select-text [overflow-wrap:anywhere]">
                          <UserMemoStreamdown text={collapsePromptArtifacts(turn.userText)} />
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
                            <AssistantStatus
                              key={`step-${stepIdx}`}
                              mode={step.done ? "thought" : "thinking"}
                              thinkingText={thinkingText}
                            />
                          );
                        }

                        if (step.type === "text") {
                          const isLastTurn = turnIdx === visibleTurns.length - 1;
                          const isLastStep = stepIdx === turn.steps.length - 1;
                          const isAnimating = view.streaming && isLastTurn && isLastStep;

                          return (
                            <div
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
                    (!view.streaming || turnIdx !== visibleTurns.length - 1) &&
                    (turn.assistantTs ?? turn.ts) != null && (
                      <div className="-mt-4">
                        <TurnFooter
                          align="start"
                          ts={(turn.assistantTs ?? turn.ts) as number}
                          copyText={assistantCopyText(turn)}
                          onRetry={
                            turnIdx === visibleTurns.length - 1
                              ? () => void retryTurn()
                              : undefined
                          }
                        />
                      </div>
                    )}
                </div>
              ))}

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
            <div ref={endRef} />
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
