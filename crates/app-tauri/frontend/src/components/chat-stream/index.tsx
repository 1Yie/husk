// ChatStream — matches gensei's web ConversationThread design 1:1.
// Replaces legacy app-desktop timeline dots, lines, and avatar cards.

import { memo, useEffect, useRef, useState, useCallback, useMemo, type ComponentType } from "react";
import { cjk } from "@streamdown/cjk";
import { Streamdown } from "streamdown";
import type { SessionView, StreamItem } from "../../hooks/stream-view";
import { AssistantStatus } from "../assistant-status";
import { ChatSkeleton } from "../chat-skeleton";
import { ToolChips, type ToolChipRow } from "../tool-chips";
import {
  Check,
  Copy,
  ArrowDown,
  Moon,
  MoonStar,
  Sun,
  SunDim,
  SunMedium,
  Sunrise,
  Sunset,
  FileCode,
  Terminal,
  Sparkles,
  type IconProps,
} from "@keyline-icons/react";
import { codexMarkdownComponents } from "./markdown-components";
import { TooltipSimple } from "@/components/ui/tooltip";
import { prismCodePlugin } from "../../lib/syntax-highlight";
import { ChatTurnRail, type RailMark } from "./chat-turn-rail";
import { cn } from "@/lib/utils";

const streamdownIcons = {
  CheckIcon: Check,
  CopyIcon: Copy,
};

/** Streamdown wrapped in `memo` — finished turns' `text` strings are
 * stable references (applyEvent only concatenates the streaming tail),
 * so the whole transcript doesn't re-parse + re-highlight markdown on
 * every delta. Only the block whose text actually changed re-renders. */
const MemoStreamdown = memo(function MemoStreamdown({
  text,
  animating,
}: {
  text: string;
  animating?: boolean;
}) {
  return (
    <Streamdown
      isAnimating={animating}
      plugins={{ cjk, code: prismCodePlugin as any }}
      shikiTheme={["github-dark", "github-dark"]}
      components={codexMarkdownComponents}
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

/** Interactive token chip in user chat bubble distinguishing @, /, and $ mentions. */
function UserTokenChip({ token }: { token: string }) {
  const clean = token.trim();

  if (clean.startsWith("@")) {
    const fullPath = clean.slice(1);
    const fileName = fullPath.split("/").filter(Boolean).pop() || fullPath;
    return (
      <TooltipSimple
        content={
          <div className="flex flex-col gap-1 text-left max-w-xs break-all py-0.5">
            <div className="flex items-center gap-1.5">
              <FileCode className="h-3.5 w-3.5 text-blue-400 shrink-0" />
              <span className="font-semibold text-neutral-100">{fileName}</span>
            </div>
            <div className="font-mono text-[11px] text-neutral-300 bg-neutral-950/70 rounded px-1.5 py-0.5 border border-neutral-700/60">
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
              <Terminal className="h-3.5 w-3.5 text-violet-400 shrink-0" />
              <span className="font-semibold text-neutral-100">指令 /{cmdName}</span>
            </div>
            {desc && <span className="text-[11.5px] text-neutral-300">{desc}</span>}
            {args && (
              <div className="font-mono text-[11px] text-neutral-400 bg-neutral-950/70 rounded px-1.5 py-0.5 border border-neutral-700/60">
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
              <Sparkles className="h-3.5 w-3.5 text-amber-400 shrink-0" />
              <span className="font-semibold text-neutral-100">技能 ${skillName}</span>
            </div>
            {args && (
              <div className="font-mono text-[11px] text-neutral-400 bg-neutral-950/70 rounded px-1.5 py-0.5 border border-neutral-700/60">
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
  ...codexMarkdownComponents,
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
        className="bg-black/[0.05] dark:bg-white/[0.08] text-neutral-800 dark:text-neutral-200 px-1.5 py-[1px] rounded-[4px] text-[12px] font-mono border border-black/[0.07] dark:border-white/[0.08] font-normal mx-0.5 inline-block leading-snug align-baseline"
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

/** Time-of-day greeting + matching keyline icon for the empty-session
 * hero — the "今天想做点什么?" style of Claude/ChatGPT new chats. */
function timeGreeting(): {
  label: string;
  Icon: ComponentType<IconProps>;
  tone: string;
} {
  const h = new Date().getHours();
  if (h < 5) return { label: "夜深了", Icon: MoonStar, tone: "text-indigo-400 dark:text-indigo-300" };
  if (h < 9) return { label: "早上好", Icon: Sunrise, tone: "text-amber-500 dark:text-amber-400" };
  if (h < 12) return { label: "上午好", Icon: Sun, tone: "text-amber-500 dark:text-amber-400" };
  if (h < 14) return { label: "中午好", Icon: SunMedium, tone: "text-orange-500 dark:text-orange-400" };
  if (h < 18) return { label: "下午好", Icon: SunDim, tone: "text-amber-600 dark:text-amber-500" };
  if (h < 20) return { label: "傍晚好", Icon: Sunset, tone: "text-orange-500 dark:text-orange-400" };
  return { label: "晚上好", Icon: Moon, tone: "text-indigo-400 dark:text-indigo-300" };
}

/** Empty state for a fresh session — a large centered greeting plus a
 * hint pointing at the composer, replacing the old one-line caption. */
function EmptyGreeting() {
  const { label, Icon, tone } = timeGreeting();
  return (
    <div className="my-auto flex flex-col items-center gap-3 select-none pb-16">
      <span className="flex items-center gap-3 text-[26px] leading-snug font-medium tracking-tight text-neutral-800 dark:text-neutral-100">
        <Icon size={24} className={tone} />
        {label}，今天想做点什么？
      </span>
      <span className="text-[13px] text-neutral-400 dark:text-neutral-500">
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
}

type AssistantStep =
  | { type: "thinking"; text: string; done: boolean }
  | { type: "text"; text: string; streaming: boolean }
  | { type: "tools"; rows: ToolChipRow[] }
  | { type: "system"; text: string };

interface Turn {
  id: string;
  userText?: string;
  /** Epoch ms of the user message — drives the `—— time ——` divider. */
  ts?: number;
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

function parseTurns(items: StreamItem[]): Turn[] {
  const turns: Turn[] = [];
  let currentTurn: Turn | null = null;

  const ensureTurn = () => {
    if (!currentTurn) {
      currentTurn = { id: `turn-${turns.length}`, steps: [] };
      turns.push(currentTurn);
    }
    return currentTurn;
  };

  for (let idx = 0; idx < items.length; idx++) {
    const item = items[idx];

    if (item.kind === "user") {
      currentTurn = {
        id: `turn-${turns.length}-${idx}`,
        userText: item.text,
        ts: item.ts,
        steps: [],
      };
      turns.push(currentTurn);
      continue;
    }

    const t = ensureTurn();
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

/** Turns mounted on first paint — long sessions render their tail first;
 * the top sentinel expands the window as the user scrolls up. A full
 * mount of a 200-turn transcript is the multi-second jank this avoids. */
const TURN_PAGE = 30;

export function ChatStream({ view, bottomPad = 128, composerH, loading, sessionKey }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const endRef = useRef<HTMLDivElement>(null);
  const pinnedRef = useRef(true);
  const [isAtBottom, setIsAtBottom] = useState(true);

  // Incremental mount window — how many trailing turns are in the DOM.
  // `sessionKey` resets it so switching sessions starts at the tail again.
  const [turnLimit, setTurnLimit] = useState(TURN_PAGE);
  useEffect(() => {
    setTurnLimit(TURN_PAGE);
    // Session switch must land at the newest message — the scroll
    // container is reused across sessions, so its scrollTop and the
    // pinned flag would otherwise keep whatever position the previous
    // session left (that's why some switches opened at the oldest turn).
    pinnedRef.current = true;
    setIsAtBottom(true);
    requestAnimationFrame(() => {
      const el = scrollRef.current;
      if (el) el.scrollTo({ top: el.scrollHeight, behavior: "instant" });
    });
  }, [sessionKey]);

  const scrollToBottom = () => {
    pinnedRef.current = true;
    setIsAtBottom(true);
    isSmoothScrollingRef.current = true;
    const el = scrollRef.current;
    if (el) {
      el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
      setTimeout(() => {
        isSmoothScrollingRef.current = false;
      }, 500);
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

  // The render window — trailing `turnLimit` turns. Older turns mount
  // progressively via the top sentinel instead of all at once.
  const visibleTurns =
    turns.length > turnLimit ? turns.slice(turns.length - turnLimit) : turns;
  const hiddenTurns = turns.length - visibleTurns.length;

  // Top sentinel — scrolling into it grows the window by one page. Rate-
  // capped so a fast fling spreads the mounts across frames instead of
  // re-creating the full-mount jank it exists to avoid.
  const sentinelRef = useRef<HTMLDivElement>(null);
  const lastExpandRef = useRef(0);
  useEffect(() => {
    const root = scrollRef.current;
    const target = sentinelRef.current;
    if (!root || !target || hiddenTurns <= 0) return;
    const obs = new IntersectionObserver(
      (entries) => {
        if (!entries.some((e) => e.isIntersecting)) return;
        const now = performance.now();
        if (now - lastExpandRef.current < 250) return;
        lastExpandRef.current = now;
        setTurnLimit((c) => c + TURN_PAGE);
      },
      { root, rootMargin: "300px" }
    );
    obs.observe(target);
    return () => obs.disconnect();
  }, [hiddenTurns > 0, sessionKey]);

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

    turns.forEach((turn, idx) => {
      const turnNum = idx + 1;
      if (turn.userText) {
        list.push({
          id: `mark-${turn.id}-user`,
          type: "user",
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
          targetId: `chat-turn-${turn.id}-assistant`,
          previewTitle: `助手回复 #${turnNum}${isStreaming ? " (生成中)" : ""}`,
          previewSnippet: getAssistantPreview(turn.steps),
          isStreaming,
        });
      } else if (idx === turns.length - 1 && (view.streaming || showReplyWait)) {
        list.push({
          id: `mark-${turn.id}-assistant-waiting`,
          type: "assistant",
          targetId: "chat-reply-wait",
          previewTitle: `助手回复 #${turnNum} (等待回复)`,
          previewSnippet: "正在等待回复...",
          isStreaming: true,
        });
      }
    });

    return list;
  }, [turns, view.streaming, showReplyWait]);

  const [activeMarkId, setActiveMarkId] = useState<string>("mark-top");
  const isSmoothScrollingRef = useRef(false);

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

    const containerTop = el.getBoundingClientRect().top;
    const triggerY = containerTop + 100;

    let currentId = marks[0].id;
    for (const mark of marks) {
      if (mark.type === "top") continue;
      const target = document.getElementById(mark.targetId);
      if (target) {
        const rect = target.getBoundingClientRect();
        if (rect.top <= triggerY) {
          currentId = mark.id;
        } else {
          break;
        }
      }
    }
    setActiveMarkId(currentId);
  }, [marks]);

  useEffect(() => {
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

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
    pinnedRef.current = atBottom;
    setIsAtBottom(atBottom);
    updateActiveMark();
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

    // The target may sit outside the mounted window — grow it to include
    // the turn, then scroll once the DOM lands (two frames: state →
    // render → layout).
    const turnMatch = mark.targetId.match(/^chat-turn-(turn-\d+)-/);
    if (turnMatch) {
      const idx = turns.findIndex((t) => t.id === turnMatch[1]);
      const firstVisible = turns.length - visibleTurns.length;
      if (idx >= 0 && idx < firstVisible) {
        setTurnLimit(turns.length - idx);
        isSmoothScrollingRef.current = true;
        requestAnimationFrame(() =>
          requestAnimationFrame(() => {
            document
              .getElementById(mark.targetId)
              ?.scrollIntoView({ behavior: "smooth", block: "start" });
            setTimeout(() => {
              isSmoothScrollingRef.current = false;
            }, 500);
          })
        );
        return;
      }
    }

    const el = document.getElementById(mark.targetId);
    if (el) {
      isSmoothScrollingRef.current = true;
      el.scrollIntoView({ behavior: "smooth", block: "start" });
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

  return (
    <div className="bg-white dark:bg-[#141416] flex-1 min-h-0 w-full flex flex-col select-text relative">
      <ChatTurnRail
        marks={marks}
        activeId={activeMarkId}
        onSelectMark={handleSelectMark}
        onDragScroll={handleDragScroll}
      />

      <div className="stream-scroll" ref={scrollRef} onScroll={onScroll}>
        <div className="max-w-3xl w-full mx-auto px-4 pt-6 flex flex-col gap-6 min-h-full">
          <div id="chat-stream-top" className="h-0 w-full" />
          {loading ? (
            <ChatSkeleton />
          ) : view.items.length === 0 && !view.streaming ? (
            <EmptyGreeting />
          ) : null}

          {hiddenTurns > 0 && (
            <div
              ref={sentinelRef}
              className="flex items-center justify-center py-2 text-[11px] text-neutral-400 select-none"
              aria-hidden="true"
            >
              加载更早的消息…
            </div>
          )}

          {visibleTurns.map((turn, turnIdx) => (
            <div
              key={turn.id}
              className="flex w-full flex-col gap-6"
              style={{ contentVisibility: "auto", containIntrinsicSize: "auto 320px" }}
            >
              {turn.ts != null && (
                <div
                  className="flex items-center gap-3 select-none"
                  aria-hidden="true"
                  data-tooltip={formatFullTime(turn.ts)}
                >
                  <div className="h-px flex-1 bg-neutral-200/70 dark:bg-neutral-700/60" />
                  <span className="text-[11px] font-medium text-neutral-400 dark:text-neutral-500 tracking-wide">
                    {formatTurnTime(turn.ts)}
                  </span>
                  <div className="h-px flex-1 bg-neutral-200/70 dark:bg-neutral-700/60" />
                </div>
              )}
              {turn.userText && (
                <div
                  id={`chat-turn-${turn.id}-user`}
                  className="bg-neutral-100 dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 ms-auto flex w-fit max-w-[80%] flex-col gap-2 rounded-xl px-3.5 py-2.5 text-[14px]"
                >
                  <div className="min-w-0 text-[14px] leading-relaxed [&_p]:max-w-none [&_p:first-child]:mt-0 [&_p:last-child]:mb-0 select-text">
                    <UserMemoStreamdown text={collapsePromptArtifacts(turn.userText)} />
                  </div>
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
                          className="w-full min-w-0 text-[14px] text-neutral-900 dark:text-neutral-100 [&_p]:max-w-none"
                          key={`step-${stepIdx}`}
                        >
                          <MemoStreamdown text={step.text} animating={isAnimating} />
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
                          <ToolChips rows={step.rows} />
                        </div>
                      );
                    }

                    if (step.type === "system") {
                      return (
                        <div
                          key={`step-${stepIdx}`}
                          className="text-xs text-neutral-400 font-mono py-1 select-none"
                        >
                          {step.text}
                        </div>
                      );
                    }

                    return null;
                  })}
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
            className="w-8 h-8 rounded-full bg-white dark:bg-[#202024] hover:bg-neutral-50 dark:hover:bg-[#2a2a30] text-neutral-600 hover:text-neutral-900 dark:text-neutral-300 dark:hover:text-white flex items-center justify-center border border-neutral-200/90 dark:border-neutral-700/80 shadow-[0_2px_8px_rgba(0,0,0,0.08)] dark:shadow-[0_2px_8px_rgba(0,0,0,0.35)] transition-all active:scale-95 cursor-pointer"
          >
            <ArrowDown className="h-4 w-4" />
          </button>
        </TooltipSimple>
      </div>
    </div>
  );
}
