// Composer — floating input bar with config.toml-based model selection & Orb-driven send button.

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  Plus,
  ChevronDown,
  ChevronsUpDown,
  ArrowUp,
  Square,
  FileText,
  Terminal,
  GitCompare,
  FolderOpen,
  Search,
  ListCheck,
  Wrench,
  ShieldCheck,
  Check,
  Brain,
  Sparkles,
  Slash,
  FileCode,
  Image,
  File,
  Paperclip,
  X,
  Bot,
  Map,
  Clock,
  GripVertical,
  ListPlus,
} from "@keyline-icons/react";
import { Orb } from "../agent-orb";
import { parseTodos, type TodoItem } from "../todo-view";
import * as agent from "../../invoke/agent";
import type { Attachment, FileItem, SkillItem } from "../../invoke/agent";
import type { SessionView } from "../../hooks/stream-view";
import { pendingApprovalOf } from "../../hooks/stream-view";
import { onQuoteRequest } from "../../lib/selection-bus";

export function extractLatestTodos(view: SessionView): {
  items: TodoItem[];
  doneCount: number;
  totalCount: number;
  allDone: boolean;
} | null {
  if (!view || !view.items || view.items.length === 0) return null;
  for (let i = view.items.length - 1; i >= 0; i--) {
    const it = view.items[i];
    if (it.kind === "tool") {
      const text = it.content || "";
      const isTodoTool =
        it.name === "todo" ||
        it.uiType === "todo" ||
        (text.includes("done") && text.includes("[")) ||
        /\[[ xX]\]\s*#?\d+/i.test(text);

      if (isTodoTool && text) {
        const parsed = parseTodos(text);
        if (parsed.items.length > 0) {
          return {
            items: parsed.items,
            doneCount: parsed.doneCount,
            totalCount: parsed.totalCount,
            allDone: parsed.totalCount > 0 && parsed.doneCount === parsed.totalCount,
          };
        }
      }
    }
  }
  return null;
}
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuItem,
  DropdownMenuGroup,
} from "@/components/ui/dropdown-menu";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger, TooltipSimple } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

const PERMISSION_MODES = [
  { value: "default", label: "默认", desc: "编辑与命令均需手动确认" },
  { value: "acceptEdits", label: "接受编辑", desc: "自动允许文件修改，终端命令仍需确认" },
  { value: "auto", label: "自动", desc: "自动执行修改与常规命令，仅高危指令确认" },
  { value: "dontAsk", label: "不询问", desc: "仅允许只读操作，拒绝所有修改与命令" },
  { value: "bypassPermissions", label: "跳过权限", desc: "完全信任，自动跳过所有确认" },
] as const;

const AGENT_MODES = [
  { value: "build", label: "构建", desc: "完整工具集 — 读写、执行、验证" },
  { value: "plan", label: "计划", desc: "只读分析，产出实施方案，批准后执行" },
  { value: "goal", label: "目标", desc: "自主推进直到目标达成或明确受阻" },
] as const;

const THINKING_LEVELS = [
  { value: "off", label: "关闭思考", desc: "不使用推理计算" },
  { value: "minimal", label: "极低强度", desc: "最小推理深度" },
  { value: "low", label: "低强度", desc: "轻度思考分析" },
  { value: "medium", label: "中等思考", desc: "标准平衡思考" },
  { value: "high", label: "高强度", desc: "深度推演与设计" },
  { value: "xhigh", label: "超高强度", desc: "超深层推演" },
  { value: "max", label: "最大思考", desc: "最大算力深度推演" },
] as const;

/** Tool name → icon for the approval strip. */
function toolIcon(name: string) {
  const cls = "h-3.5 w-3.5";
  if (name === "bash" || name === "pty" || name === "shell") return <Terminal className={cls} />;
  if (name === "fuzzy_patch" || name === "apply_patch") return <GitCompare className={cls} />;
  if (name === "write_file" || name === "fs_patch") return <FileText className={cls} />;
  if (name === "list_dir" || name === "read_dir") return <FolderOpen className={cls} />;
  if (name === "grep" || name === "smart_read" || name === "fs_read")
    return <Search className={cls} />;
  if (name === "todo") return <ListCheck className={cls} />;
  return <Wrench className={cls} />;
}

export function ComposerBar({
  view,
  workspaceRoot,
  sessionKey,
}: {
  view: SessionView;
  workspaceRoot?: string;
  /** `root:id` — queued drafts belong to a session; switching drops them. */
  sessionKey?: string;
}) {
  const [text, setText] = useState("");
  const [mode, setMode] = useState<string>("default");
  // Agent mode (build/plan/goal) — orthogonal to the permission mode: the
  // gate decides HOW calls get approved, the mode decides WHAT the session
  // may do at all (plan = readonly registry, goal = completion contract).
  const [agentMode, setAgentMode] = useState<string>("build");
  // Queued follow-ups — submitting mid-turn parks the payload here instead
  // of steering; each drains as a fresh prompt the moment the turn ends.
  // "立即发送" on a row promotes it to a live Steer.
  const [queued, setQueued] = useState<string[]>([]);
  useEffect(() => setQueued([]), [sessionKey]);
  const [todoCollapsed, setTodoCollapsed] = useState(false);
  // Files attached via the `+` button — chips above the textarea; their
  // content rides inside the prompt text as fenced blocks (same channel
  // `@` mention expansion uses, so the kernel needs no new IPC).
  const [attachments, setAttachments] = useState<Attachment[]>([]);

  const [activeModel, setActiveModel] = useState<string>("");
  const [activeProvider, setActiveProvider] = useState<string>("");
  const [models, setModels] = useState<agent.ModelItem[]>([]);
  const [activeThinkingLevel, setActiveThinkingLevel] = useState<string>("off");

  // Right-click in the chat stream ("就选中内容提问" / "粘贴到输入框")
  // lands here via the selection bus. A passage arrives as a quoted
  // blockquote so the model reads it as referenced context, then the
  // textarea takes focus so the question can be typed underneath.
  useEffect(() => {
    return onQuoteRequest((text) => {
      const quote = `> ${text.trim().replace(/\n/g, "\n> ")}\n\n`;
      setText((prev) => (prev ? `${quote}${prev}` : quote));
      requestAnimationFrame(() => {
        const el = document.querySelector<HTMLTextAreaElement>("textarea[data-composer]");
        el?.focus();
        el?.setSelectionRange(el.value.length, el.value.length);
      });
    });
  }, []);

  const fetchModels = async () => {
    try {
      const info = await agent.getModelInfo();
      if (info) {
        if (info.active_model) setActiveModel(info.active_model);
        if (info.active_provider) setActiveProvider(info.active_provider);
        if (info.active_thinking_level) setActiveThinkingLevel(info.active_thinking_level);
        if (info.active_permission_mode) setMode(info.active_permission_mode);
        if (info.active_agent_mode) setAgentMode(info.active_agent_mode);
        if (info.models && Array.isArray(info.models)) {
          setModels(info.models);
        }
      }
    } catch (err) {
      console.warn("Could not load model_info from config:", err);
    }
  };

  useEffect(() => {
    void fetchModels();
  }, [workspaceRoot]);

  const currentModelObj =
    models.find((m) => m.model === activeModel && m.provider === activeProvider) ||
    models.find((m) => m.model === activeModel);

  const hasReasoning = Boolean(
    currentModelObj?.reasoning ||
    (currentModelObj?.available_levels && currentModelObj.available_levels.length > 0),
  );

  const availableLevelValues =
    currentModelObj?.available_levels && currentModelObj.available_levels.length > 0
      ? currentModelObj.available_levels
      : hasReasoning
        ? ["off", "low", "medium", "high", "max"]
        : [];

  const effectiveThinkingLevels = THINKING_LEVELS.filter((lvl) =>
    availableLevelValues.includes(lvl.value),
  );

  const currentThinkingObj = THINKING_LEVELS.find((l) => l.value === activeThinkingLevel);
  const currentThinkingLabel = currentThinkingObj?.label || activeThinkingLevel;

  const handleSelectModel = async (provider: string, model: string) => {
    setActiveModel(model);
    setActiveProvider(provider);
    try {
      await agent.setModel(provider, model);
      const target = models.find((m) => m.model === model && m.provider === provider);
      if (target?.available_levels && target.available_levels.length > 0) {
        if (!target.available_levels.includes(activeThinkingLevel)) {
          const nextLevel = target.available_levels.includes("medium")
            ? "medium"
            : target.available_levels.find((l) => l !== "off") || "off";
          setActiveThinkingLevel(nextLevel);
          await agent.setThinkingLevel(nextLevel);
        }
      }
    } catch (err) {
      console.error("setModel failed:", err);
    }
  };

  const handleSelectThinkingLevel = async (level: string) => {
    setActiveThinkingLevel(level);
    try {
      await agent.setThinkingLevel(level);
    } catch (err) {
      console.error("setThinkingLevel failed:", err);
    }
  };

  // ---- @file / /command / $skill mention popup --------------------------
  //
  // The textarea stays a plain controlled input — the popup reads the token
  // under the caret (whitespace-bounded) and rewrites it on accept. `/@/$`
  // are interchangeable triggers; what they search differs.

  type Mention = { trigger: "@" | "/" | "$"; query: string; start: number };
  const [mention, setMention] = useState<Mention | null>(null);
  const [mentionIndex, setMentionIndex] = useState(0);
  const [files, setFiles] = useState<FileItem[]>([]);
  const [skills, setSkills] = useState<SkillItem[]>([]);

  // Recompute the active mention token from the current text + caret.
  // Returns null when the caret isn't inside a `/@/$` token.
  const detectMention = (value: string, caret: number): Mention | null => {
    const upto = value.slice(0, caret);
    // Token = last whitespace-delimited word before the caret. The query may
    // itself contain `/`/`@`/`$` — file paths like `@src/main` or `@pkg/@scope`
    // must keep the popup open while filtering.
    const m = /(?:^|\s)([@/$])(\S*)$/.exec(upto);
    if (!m) return null;
    const trigger = m[1] as Mention["trigger"];
    const query = m[2];
    const start = caret - query.length - 1;
    // `@` and `$` open their pickers anywhere in the text — the kernel
    // inlines them mid-prompt (`expand_user_tokens`). `/` only opens at
    // the very start: mid-sentence it's a path separator, and the kernel
    // dispatches only a *leading* `/x`.
    if (trigger === "/" && start !== 0) return null;
    return { trigger, query, start };
  };

  const refreshMention = (value: string, caret: number) => {
    setMention(detectMention(value, caret));
    setMentionIndex(0);
  };

  const mentionQuery = mention?.query;
  const mentionTrigger = mention?.trigger;
  useEffect(() => {
    if (mentionTrigger === undefined) return;
    let dead = false;
    if (mentionTrigger === "@") {
      agent
        .listFiles(mentionQuery)
        .then((r) => {
          if (!dead) setFiles(r);
        })
        .catch(() => {
          if (!dead) setFiles([]);
        });
    } else {
      // `/` and `$` both surface the skill list; `/` prepends built-ins.
      agent
        .listSkills()
        .then((r) => {
          if (!dead) setSkills(r);
        })
        .catch(() => {
          if (!dead) setSkills([]);
        });
    }
    return () => {
      dead = true;
    };
  }, [mentionTrigger, mentionQuery]);

  // Static slash commands — dispatched by the kernel's CommandRegistry.
  const BUILTIN_COMMANDS = useMemo(
    () => [
      { name: "clear", desc: "清空会话历史" },
      { name: "compact", desc: "压缩上下文以释放 token" },
      { name: "undo", desc: "回滚上一轮的文件修改" },
      { name: "model", desc: "切换模型 /model <provider>/<model>" },
      { name: "diff", desc: "查看本轮改动的文件" },
      { name: "skills", desc: "列出工作区可用 skills" },
    ],
    [],
  );

  // The rendered rows — what arrow keys + Enter act on.
  const mentionRows = useMemo(() => {
    if (!mention)
      return [] as {
        key: string;
        label: string;
        hint: string;
        icon: "file" | "cmd" | "skill";
        insert: string;
      }[];
    const q = mention.query.toLowerCase();
    if (mention.trigger === "@") {
      return files.map((f) => ({
        key: `@${f.path}`,
        label: f.name,
        hint: f.path,
        icon: "file" as const,
        insert: `@${f.path}`,
      }));
    }
    const rows: {
      key: string;
      label: string;
      hint: string;
      icon: "cmd" | "skill";
      insert: string;
    }[] = [];
    if (mention.trigger === "/") {
      for (const c of BUILTIN_COMMANDS) {
        if (q && !c.name.startsWith(q)) continue;
        rows.push({
          key: `/${c.name}`,
          label: `/${c.name}`,
          hint: c.desc,
          icon: "cmd",
          insert: `/${c.name}`,
        });
      }
    }
    for (const s of skills) {
      if (q && !s.name.toLowerCase().includes(q)) continue;
      rows.push({
        key: `$${s.name}`,
        label: `$${s.name}`,
        hint: s.description || s.path,
        icon: "skill",
        // `$`-triggered picks must insert the `$` token — a `/name` typed
        // mid-text is just a literal (the kernel only dispatches a
        // *leading* `/x`), while `$name` expands anywhere.
        insert: mention.trigger === "$" ? `$${s.name}` : `/${s.name}`,
      });
    }
    return rows;
  }, [mention, files, skills, BUILTIN_COMMANDS]);

  const acceptMention = (row: { insert: string }) => {
    if (!mention) return;
    const el = document.querySelector<HTMLTextAreaElement>("textarea[data-composer]");
    const caret = el?.selectionStart ?? text.length;
    const next = text.slice(0, mention.start) + row.insert + " " + text.slice(caret);
    setText(next);
    setMention(null);
    requestAnimationFrame(() => {
      el?.focus();
      const pos = mention.start + row.insert.length + 1;
      el?.setSelectionRange(pos, pos);
    });
  };

  const streaming = view.streaming;

  const pick = async (kind: "image" | "text" | "any") => {
    try {
      const paths = await agent.pickAttachments(kind);
      for (const p of paths) {
        const a = await agent.readAttachment(p).catch(() => null);
        if (a) {
          setAttachments((prev) =>
            prev.some((x) => x.path === a.path) ? prev : [...prev, a],
          );
        }
      }
    } catch (e) {
      console.error("pick attachment failed:", e);
    }
  };

  const submit = async () => {
    const trimmed = text.trim();
    if (!trimmed && attachments.length === 0) return;
    // Inline attachments into the prompt — text files as fenced blocks
    // (mirrors the kernel's `@` mention expansion); images emit an
    // `<attached-image>` marker the kernel turns into real image parts
    // when the model declares vision (path is the staged workspace copy,
    // so sandboxed tools can see it too); binaries stay path references.
    const blocks = attachments
      .map((a) =>
        a.kind === "text" && a.content != null
          ? `\n\n<attached-file path="${a.path}">\n\`\`\`\n${a.content}${a.truncated ? "\n… (truncated)" : ""}\n\`\`\``
          : a.kind === "image"
            ? `\n\n<attached-image path="${a.path}" name="${a.name.replace(/"/g, "")}"/>`
            : `\n\n[attached binary: ${a.path} — binary file, not inlined]`,
      )
      .join("");
    const payload = `${trimmed}${blocks}`;
    setText("");
    setAttachments([]);
    if (streaming) {
      // Queue it — delivered as a fresh Prompt the moment this turn ends.
      // Mid-turn steering is still one click away on the queued row.
      setQueued((q) => [...q, payload]);
      return;
    }
    try {
      await agent.sendPrompt(payload);
    } catch (e) {
      console.error("agent_cmd failed:", e);
      setText(trimmed);
    }
  };

  /** Promote a queued draft to a live Steer — delivered mid-turn. */
  const steerNow = async (i: number) => {
    const item = queued[i];
    setQueued((q) => q.filter((_, j) => j !== i));
    try {
      await agent.steer(item);
    } catch (e) {
      console.error("steer failed:", e);
      setQueued((q) => [item, ...q]);
    }
  };

  const removeQueued = (i: number) =>
    setQueued((q) => q.filter((_, j) => j !== i));

  // Drag-to-reorder — plain HTML5 DnD on the row; `overIdx` paints the
  // drop line. Order = send order, so the strip is the source of truth.
  const [dragIdx, setDragIdx] = useState<number | null>(null);
  const [overIdx, setOverIdx] = useState<number | null>(null);
  const moveQueued = (from: number, to: number) =>
    setQueued((q) => {
      const next = [...q];
      const [m] = next.splice(from, 1);
      next.splice(to, 0, m);
      return next;
    });

  // Drain the queue one prompt per turn end — `wasStreaming` guards the
  // transition so a state change alone can't flush the whole list.
  const wasStreamingRef = useRef(false);
  useEffect(() => {
    const was = wasStreamingRef.current;
    wasStreamingRef.current = streaming;
    if (was && !streaming && queued.length > 0) {
      const head = queued[0];
      setQueued((q) => q.slice(1));
      void agent.sendPrompt(head);
    }
  }, [streaming, queued]);

  const cancel = async () => {
    try {
      await agent.cancelTurn();
    } catch {
      /* no active turn */
    }
  };

  const pending = pendingApprovalOf(view);
  const latestTodos = useMemo(() => extractLatestTodos(view), [view.items]);
  const hasActiveTodos = Boolean(latestTodos && latestTodos.totalCount > 0 && !latestTodos.allDone);

  const [decidedId, setDecidedId] = useState<number | null>(null);
  const decide = async (approved: boolean) => {
    if (!pending || decidedId === pending.requestId) return;
    const reqId = pending.requestId;
    setDecidedId(reqId);
    try {
      await agent.decideTool(reqId, approved);
    } catch (e) {
      console.error("decideTool failed:", e);
      setDecidedId(null);
    }
  };

  const switchMode = async (m: string) => {
    setMode(m);
    try {
      await agent.setPermissionMode(m);
      if (pending) {
        const isFileTool =
          pending.toolName === "fuzzy_patch" ||
          pending.toolName === "apply_patch" ||
          pending.toolName === "write_file" ||
          pending.toolName === "fs_patch";
        if (m === "bypassPermissions" || m === "auto" || (m === "acceptEdits" && isFileTool)) {
          await decide(true);
        }
      }
    } catch (e) {
      console.error("setPermissionMode failed:", e);
    }
  };

  const switchAgentMode = async (m: string) => {
    setAgentMode(m);
    try {
      await agent.setAgentMode(m);
    } catch (e) {
      console.error("setAgentMode failed:", e);
    }
  };

  const modeLabel = PERMISSION_MODES.find((m) => m.value === mode)?.label ?? "默认";
  const agentModeLabel = AGENT_MODES.find((m) => m.value === agentMode)?.label ?? "构建";

  // Plan → build handoff: a finished plan-mode turn ends on an assistant
  // block (the plan). Approving flips to build and feeds the plan back as
  // the execution instruction — Claude Code's exit-plan-mode flow.
  const lastItem = view.items[view.items.length - 1];
  const planReady =
    agentMode === "plan" && !streaming && lastItem?.kind === "assistant";
  const approvePlan = async () => {
    await switchAgentMode("build");
    try {
      await agent.sendPrompt("上面的计划已获批准 — 按它实施。");
    } catch (e) {
      console.error("approve-plan send failed:", e);
    }
  };

  return (
    <TooltipProvider delayDuration={300}>
      {/* Since the native right-side scrollbar was removed, we use symmetric
          padding so the floating composer card is perfectly horizontally centered
          with the conversation stream. */}
      <div className="w-full px-4 pb-6 select-none pointer-events-none">
        <div className="max-w-3xl w-full mx-auto flex flex-col gap-2 pointer-events-auto">
          {planReady && (
            <div className="w-full flex items-center gap-2 rounded-2xl border border-hairline bg-panel px-3.5 py-2.5 shadow-[0_2px_12px_rgba(0,0,0,0.025)]">
              <Map className="h-3.5 w-3.5 text-neutral-500 shrink-0" />
              <span className="text-xs text-neutral-600 font-medium flex-1">
                计划已就绪 — 审核上面的方案
              </span>
              <Button
                size="sm"
                className="shrink-0 h-6 px-2.5 text-[11px] bg-neutral-900 hover:bg-neutral-800 text-white dark:text-[#fafafa] cursor-pointer"
                onClick={() => void approvePlan()}
              >
                批准并执行
              </Button>
            </div>
          )}
          {pending || hasActiveTodos || queued.length > 0 ? (
            /* Outer container with attached banner: Approval (Priority 1) or Active Todo (Priority 2) */
            <div className="w-full bg-panel rounded-[24px] pt-2.5 flex flex-col gap-2 transition-all shadow-[0_2px_12px_rgba(0,0,0,0.025)]">
              {pending ? (
                /* Priority 1: Permission approval strip */
                <div className="flex items-center gap-2 px-3 pt-0.5 text-xs text-neutral-600 font-medium select-none">
                  <span className="text-neutral-500 shrink-0 flex items-center">
                    {toolIcon(pending.toolName)}
                  </span>
                  <span className="shrink-0 text-neutral-800 font-medium">
                    允许执行
                  </span>
                  <TooltipSimple
                    content={
                      <div className="max-w-xl max-h-64 overflow-y-auto font-mono text-xs break-all whitespace-pre-wrap select-text leading-relaxed">
                        {pending.args || pending.toolName}
                      </div>
                    }
                    side="top"
                    sideOffset={8}
                    className="max-w-xl bg-[color-mix(in_srgb,var(--husk-n900)_95%,transparent)] backdrop-blur-sm border border-[color-mix(in_srgb,var(--husk-n700)_60%,transparent)] p-2.5 shadow-xl select-text"
                  >
                    <span
                      className="font-mono text-[11px] text-neutral-600 truncate min-w-0 flex-1 bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)] border border-[color-mix(in_srgb,var(--husk-black)_6%,transparent)] px-2 py-0.5 rounded cursor-pointer hover:bg-[color-mix(in_srgb,var(--husk-black)_7%,transparent)] transition-colors"
                    >
                      {pending.args || pending.toolName}
                    </span>
                  </TooltipSimple>
                  <Button
                    size="sm"
                    className="shrink-0 h-6 px-2.5 text-[11px] bg-emerald-600 hover:bg-emerald-700 text-white dark:text-[#fafafa] cursor-pointer"
                    onClick={() => void decide(true)}
                  >
                    允许
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    className="shrink-0 h-6 px-2.5 text-[11px] border-neutral-300 text-neutral-600 hover:bg-neutral-200 cursor-pointer"
                    onClick={() => void decide(false)}
                  >
                    拒绝
                  </Button>
                </div>
              ) : null}

              {!pending && hasActiveTodos ? (
                /* Attached Todo List Strip */
                <div className="flex flex-col gap-1.5 px-3 pt-0.5">
                  <div className="flex items-center justify-between text-xs text-neutral-600 font-medium select-none">
                    <div className="flex items-center gap-2 min-w-0">
                      <ListCheck className="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400 shrink-0" />
                      <span className="shrink-0 text-neutral-800 font-medium text-xs">
                        任务清单
                      </span>
                      <div className="w-16 h-1.5 rounded-full bg-neutral-200 overflow-hidden shrink-0">
                        <div
                          className="h-full bg-emerald-500 transition-all duration-300 rounded-full"
                          style={{
                            width: `${Math.round((latestTodos!.doneCount / latestTodos!.totalCount) * 100)}%`,
                          }}
                        />
                      </div>
                      <span className="text-[11px] font-mono text-neutral-500 shrink-0">
                        {latestTodos!.doneCount}/{latestTodos!.totalCount} (
                        {Math.round((latestTodos!.doneCount / latestTodos!.totalCount) * 100)}%)
                      </span>
                    </div>

                    <TooltipSimple content={todoCollapsed ? "展开任务列表" : "折叠任务列表"} side="top">
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        onClick={() => setTodoCollapsed((prev) => !prev)}
                        className="h-6 w-6 text-neutral-400 hover:text-neutral-700 cursor-pointer"
                      >
                        <ChevronDown
                          className={cn(
                            "h-3.5 w-3.5 transition-transform duration-250 ease-out",
                            todoCollapsed ? "-rotate-90" : "rotate-0",
                          )}
                        />
                      </Button>
                    </TooltipSimple>
                  </div>

                  <div
                    className="grid transition-[grid-template-rows,opacity] duration-250 ease-out w-full"
                    style={{
                      gridTemplateRows: todoCollapsed ? "0fr" : "1fr",
                      opacity: todoCollapsed ? 0 : 1,
                    }}
                  >
                    <div className="min-h-0 overflow-hidden w-full">
                      <div className="max-h-[140px] overflow-y-auto flex flex-col gap-1 pt-1 pb-1">
                        {latestTodos!.items.map((item) => (
                          <div
                            key={item.id}
                            className={cn(
                              "group flex items-center gap-2 text-xs py-0.5 px-1 rounded transition-colors",
                              item.done
                                ? "text-neutral-400"
                                : "text-neutral-700 hover:bg-[color-mix(in_srgb,var(--husk-black)_3%,transparent)]",
                            )}
                          >
                            <span className="flex h-4 w-3.5 items-center justify-center shrink-0">
                              {item.done ? (
                                <span className="inline-flex items-center justify-center w-3.5 h-3.5 rounded-full bg-emerald-500/20 text-emerald-600 dark:text-emerald-400">
                                  <Check className="h-2 w-2 stroke-[3]" />
                                </span>
                              ) : (
                                <span className="inline-flex items-center justify-center w-3.5 h-3.5 rounded-full border border-neutral-300 group-hover:border-neutral-400 transition-colors" />
                              )}
                            </span>
                            <span className="font-mono text-[10.5px] text-neutral-400 shrink-0 select-none">
                              #{item.id}
                            </span>
                            <span className={cn("truncate flex-1 min-w-0", item.done && "line-through opacity-75")}>
                              {item.text}
                            </span>
                          </div>
                        ))}
                      </div>
                    </div>
                  </div>
                </div>
              ) : null}

              {queued.length > 0 ? (
                /* Queued follow-ups — parked drafts sent when the turn ends;
                   approval-strip visual family (icon · label · chips · actions). */
                <div className="flex flex-col gap-1.5 px-3 pt-0.5">
                  <div className="flex items-center gap-2 text-xs text-neutral-600 font-medium select-none">
                    <Clock className="h-3.5 w-3.5 text-neutral-500 shrink-0" />
                    <span className="shrink-0 text-neutral-800 font-medium text-xs">
                      排队消息
                    </span>
                    <span className="text-[11px] font-mono text-neutral-500">
                      {queued.length} 条 · 回合结束后按序发送
                    </span>
                  </div>
                  {queued.slice(0, 4).map((q, i) => (
                    <div
                      key={i}
                      draggable
                      onDragStart={(e) => {
                        setDragIdx(i);
                        e.dataTransfer.effectAllowed = "move";
                      }}
                      onDragOver={(e) => {
                        e.preventDefault();
                        e.dataTransfer.dropEffect = "move";
                        setOverIdx(i);
                      }}
                      onDrop={(e) => {
                        e.preventDefault();
                        if (dragIdx != null && dragIdx !== i) moveQueued(dragIdx, i);
                        setDragIdx(null);
                        setOverIdx(null);
                      }}
                      onDragEnd={() => {
                        setDragIdx(null);
                        setOverIdx(null);
                      }}
                      className={cn(
                        "flex items-center gap-1.5 pl-1 pr-0 rounded-md transition-colors",
                        overIdx === i && dragIdx !== i &&
                          "bg-[color-mix(in_srgb,var(--husk-black)_6%,transparent)] ring-1 ring-inset ring-[color-mix(in_srgb,var(--husk-n400)_60%,transparent)]",
                        dragIdx === i && "opacity-40",
                      )}
                    >
                      <GripVertical className="h-3.5 w-3.5 text-neutral-300 hover:text-neutral-500 cursor-grab active:cursor-grabbing shrink-0" />
                      <span className="font-mono text-[11px] text-neutral-600 truncate min-w-0 flex-1 bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)] border border-[color-mix(in_srgb,var(--husk-black)_6%,transparent)] px-2 py-0.5 rounded select-none">
                        {q.split("\n")[0]}
                      </span>
                      <Button
                        size="sm"
                        variant="outline"
                        className="shrink-0 h-6 px-2 text-[11px] border-neutral-300 text-neutral-600 hover:bg-neutral-200 cursor-pointer"
                        onClick={() => void steerNow(i)}
                      >
                        立即发送
                      </Button>
                      <button
                        type="button"
                        aria-label="移除"
                        className="shrink-0 h-5 w-5 flex items-center justify-center rounded text-neutral-400 hover:text-neutral-700 hover:bg-neutral-200 transition-colors cursor-pointer"
                        onClick={() => removeQueued(i)}
                      >
                        <X className="h-3 w-3" />
                      </button>
                    </div>
                  ))}
                  {queued.length > 4 && (
                    <span className="pl-5 text-[11px] text-neutral-400">
                      …还有 {queued.length - 4} 条
                    </span>
                  )}
                </div>
              ) : null}

              <div className="bg-white border border-hairline rounded-[18px] p-3 flex flex-col gap-2 shadow-[0_2px_8px_rgba(0,0,0,0.02)]">
                <AttachmentChips
                  items={attachments}
                  onRemove={(path) =>
                    setAttachments((prev) => prev.filter((a) => a.path !== path))
                  }
                />
                <ComposerTextarea
                  value={text}
                  onChange={setText}
                  onSubmit={() => void submit()}
                  streaming={streaming}
                  mention={mention}
                  mentionIndex={mentionIndex}
                  setMentionIndex={setMentionIndex}
                  mentionRows={mentionRows}
                  onRefreshMention={refreshMention}
                  onAcceptMention={acceptMention}
                  onDismissMention={() => setMention(null)}
                />

                <ComposerToolbar
                  agentMode={agentMode}
                  agentModeLabel={agentModeLabel}
                  switchAgentMode={switchAgentMode}
                  modeLabel={modeLabel}
                  mode={mode}
                  switchMode={switchMode}
                  activeModel={activeModel}
                  activeProvider={activeProvider}
                  models={models}
                  onSelectModel={handleSelectModel}
                  hasReasoning={hasReasoning}
                  activeThinkingLevel={activeThinkingLevel}
                  currentThinkingLabel={currentThinkingLabel}
                  effectiveThinkingLevels={effectiveThinkingLevels}
                  onSelectThinkingLevel={handleSelectThinkingLevel}
                  streaming={streaming}
                  text={text}
                  hasAttachments={attachments.length > 0}
                  onPick={(kind) => void pick(kind)}
                  submit={submit}
                  cancel={cancel}
                />
              </div>
            </div>
          ) : (
            /* Normal — single white card when no approval and no active todos. */
            <div className="w-full bg-white border border-hairline rounded-[22px] shadow-[0_2px_12px_rgba(0,0,0,0.025)] hover:border-neutral-300 transition-all p-3 flex flex-col gap-2">
              <AttachmentChips
                items={attachments}
                onRemove={(path) =>
                  setAttachments((prev) => prev.filter((a) => a.path !== path))
                }
              />
              <ComposerTextarea
                value={text}
                onChange={setText}
                onSubmit={() => void submit()}
                streaming={streaming}
                mention={mention}
                mentionIndex={mentionIndex}
                setMentionIndex={setMentionIndex}
                mentionRows={mentionRows}
                onRefreshMention={refreshMention}
                onAcceptMention={acceptMention}
                onDismissMention={() => setMention(null)}
              />

              <ComposerToolbar
                agentMode={agentMode}
                agentModeLabel={agentModeLabel}
                switchAgentMode={switchAgentMode}
                modeLabel={modeLabel}
                mode={mode}
                switchMode={switchMode}
                activeModel={activeModel}
                activeProvider={activeProvider}
                models={models}
                onSelectModel={handleSelectModel}
                hasReasoning={hasReasoning}
                activeThinkingLevel={activeThinkingLevel}
                currentThinkingLabel={currentThinkingLabel}
                effectiveThinkingLevels={effectiveThinkingLevels}
                onSelectThinkingLevel={handleSelectThinkingLevel}
                streaming={streaming}
                text={text}
                hasAttachments={attachments.length > 0}
                onPick={(kind) => void pick(kind)}
                submit={submit}
                cancel={cancel}
              />
            </div>
          )}
        </div>
      </div>
    </TooltipProvider>
  );
}

/** The mention popup — absolutely positioned above the textarea, driven by
 * the `mention`/`mentionIndex` state the parent keeps in sync with the
 * caret. Mouse-down (not click) so the textarea keeps focus. */
function MentionPopup({
  rows,
  active,
  onPick,
}: {
  rows: {
    key: string;
    label: string;
    hint: string;
    icon: "file" | "cmd" | "skill";
    insert: string;
  }[];
  active: number;
  onPick: (row: { insert: string }) => void;
}) {
  const listRef = useRef<HTMLDivElement | null>(null);
  // Keep the active row visible on ↑/↓ — done manually instead of
  // scrollIntoView so ancestor scrollers (the chat stream) never move.
  useEffect(() => {
    const list = listRef.current;
    const row = list?.children[active] as HTMLElement | undefined;
    if (!list || !row) return;
    if (row.offsetTop < list.scrollTop) {
      list.scrollTop = row.offsetTop;
    } else if (row.offsetTop + row.offsetHeight > list.scrollTop + list.clientHeight) {
      list.scrollTop = row.offsetTop + row.offsetHeight - list.clientHeight;
    }
  }, [active]);

  if (rows.length === 0) {
    return (
      <div className="absolute bottom-full left-2 right-2 mb-2 z-50 rounded-xl border border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)] bg-white shadow-popup px-3 py-2.5 text-xs text-neutral-400">
        无匹配项
      </div>
    );
  }
  return (
    <div
      className="absolute bottom-full left-2 right-2 mb-2 z-50 rounded-xl border border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)] bg-white shadow-popup overflow-hidden"
      role="listbox"
    >
      <div ref={listRef} className="max-h-64 overflow-y-auto">
        {rows.map((r, i) => (
          <button
            key={r.key}
            type="button"
            role="option"
            aria-selected={i === active}
            onMouseDown={(e) => {
              e.preventDefault();
              onPick(r);
            }}
            className={cn(
              "w-full flex items-center gap-2.5 px-3 py-2 text-left text-[12.5px] transition-colors",
              i === active
                ? "bg-neutral-100"
                : "hover:bg-neutral-50",
            )}
          >
            <span className="shrink-0 text-neutral-400">
              {r.icon === "file" ? (
                <FileCode className="h-3.5 w-3.5" />
              ) : r.icon === "skill" ? (
                <Sparkles className="h-3.5 w-3.5" />
              ) : (
                <Terminal className="h-3.5 w-3.5" />
              )}
            </span>
            <span className="font-mono font-medium text-neutral-800 shrink-0">
              {r.label}
            </span>
            <span className="truncate text-[11px] text-neutral-400">
              {r.hint}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}

/** Chip background per token kind — painted by the composer mirror layer.
 * No horizontal padding: any extra width would desync the mirror from the
 * textarea glyphs underneath. */
const TOKEN_CHIP_CLS = {
  file: "rounded-[4px] bg-blue-500/15 text-blue-700 dark:bg-blue-400/15 dark:text-blue-300",
  cmd: "rounded-[4px] bg-violet-500/15 text-violet-700 dark:bg-violet-400/15 dark:text-violet-300",
  skill: "rounded-[4px] bg-amber-500/15 text-amber-700 dark:bg-amber-400/15 dark:text-amber-300",
} as const;

/** Render composer text with `@file`/`/cmd`/`$skill` tokens as chips.
 * Mirrors the kernel's `expand_user_tokens` rules: `@` and `$` tokens
 * anywhere (whitespace/start bounded — mid-text `$name` inlines the
 * skill body), `/` only at position 0 where it's a real command. */
function highlightComposerTokens(text: string) {
  const nodes: (string | JSX.Element)[] = [];
  // `\S+` tokens — the kernel's `@`/`/`/`$` tokens run to the next
  // whitespace, so paths containing `/`, `$`, or even `@` mid-token
  // stay one chip. `(?!x)` rejects doubled triggers (`@@`, `//`, `$$`),
  // matching the kernel which re-scans past the literal first char.
  const re = /@(?!@)\S+|\$(?!\$)\S+|\/(?!\/)\S+/g;
  let m: RegExpExecArray | null;
  let last = 0;
  let key = 0;
  while ((m = re.exec(text))) {
    const token = m[0];
    const ch = token[0];
    const start = m.index;
    if (ch === "@" || ch === "$") {
      // `a@b.com` / `x$HOME` stay literal — need a whitespace/start
      // boundary, or a doubled trigger the kernel re-scans (`@@`, `$$`).
      if (start > 0 && !/\s/.test(text[start - 1]) && text[start - 1] !== ch)
        continue;
      // `$5`/`$(x)` can't be skill names — the kernel skips the lookup.
      if (ch === "$" && !/^[A-Za-z_]/.test(token.slice(1))) continue;
    } else if (start !== 0) {
      continue;
    }
    if (token.length <= 1) continue; // bare trigger char
    nodes.push(text.slice(last, start));
    const kind = ch === "@" ? "file" : ch === "$" ? "skill" : "cmd";
    nodes.push(
      <span key={key++} className={TOKEN_CHIP_CLS[kind]}>
        {token}
      </span>,
    );
    last = start + token.length;
  }
  nodes.push(text.slice(last));
  return nodes;
}

/** Attached-file chips above the textarea — icon by kind, ✕ removes.
 * Content itself never enters the textarea; it's inlined into the
 * prompt at submit time. */
function AttachmentChips({
  items,
  onRemove,
}: {
  items: Attachment[];
  onRemove: (path: string) => void;
}) {
  if (items.length === 0) return null;
  return (
    <div className="flex flex-wrap gap-1.5 pb-1">
      {items.map((a) => (
        <span
          key={a.path}
          title={a.path}
          className="inline-flex items-center gap-1.5 max-w-[240px] pl-1.5 pr-1 py-1 rounded-md bg-neutral-100 border border-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] text-[11.5px] text-neutral-700"
        >
          <span className="shrink-0 text-neutral-400">
            {a.kind === "image" && a.data_url ? (
              <img
                src={a.data_url}
                alt={a.name}
                className="h-4 w-4 rounded-sm object-cover"
              />
            ) : a.kind === "image" ? (
              <Image className="h-3.5 w-3.5" />
            ) : a.kind === "text" ? (
              <FileText className="h-3.5 w-3.5" />
            ) : (
              <File className="h-3.5 w-3.5" />
            )}
          </span>
          <span className="truncate">{a.name}</span>
          <button
            type="button"
            onClick={() => onRemove(a.path)}
            className="shrink-0 w-4 h-4 rounded-full flex items-center justify-center text-neutral-400 hover:text-neutral-700 hover:bg-neutral-200 cursor-pointer"
            aria-label={`移除 ${a.name}`}
          >
            <X className="h-2.5 w-2.5" />
          </button>
        </span>
      ))}
    </div>
  );
}

/** Textarea + mention popup — owns the `@`/`/`/`$` detection and keyboard
 * nav, reports the final text upward. Extracted so both composer layouts
 * (plain + with-banner) share one implementation. */
function ComposerTextarea({
  value,
  onChange,
  onSubmit,
  streaming,
  mention,
  mentionIndex,
  setMentionIndex,
  mentionRows,
  onRefreshMention,
  onAcceptMention,
  onDismissMention,
}: {
  value: string;
  onChange: (v: string) => void;
  onSubmit: () => void;
  streaming: boolean;
  mention: { trigger: "@" | "/" | "$"; query: string; start: number } | null;
  mentionIndex: number;
  setMentionIndex: (i: number) => void;
  mentionRows: {
    key: string;
    label: string;
    hint: string;
    icon: "file" | "cmd" | "skill";
    insert: string;
  }[];
  onRefreshMention: (value: string, caret: number) => void;
  onAcceptMention: (row: { insert: string }) => void;
  onDismissMention: () => void;
}) {
  const mirrorRef = useRef<HTMLDivElement | null>(null);
  const taRef = useRef<HTMLTextAreaElement | null>(null);
  const open = mention !== null;

  // Auto-grow — the textarea starts one line tall and stretches with
  // content up to 180px, then scrolls. The mirror is `absolute inset-0`
  // on the same relative wrapper, so it follows the height for free;
  // scroll sync still rides `onScroll` below.
  const MAX_HEIGHT = 180;
  useLayoutEffect(() => {
    const el = taRef.current;
    if (!el) return;
    el.style.height = "0px";
    const next = Math.min(el.scrollHeight, MAX_HEIGHT);
    el.style.height = `${Math.max(next, 42)}px`;
    el.style.overflowY = el.scrollHeight > MAX_HEIGHT ? "auto" : "hidden";
    // Mirror must track the new top scroll position after the resize.
    if (mirrorRef.current) mirrorRef.current.scrollTop = el.scrollTop;
  }, [value]);

  return (
    <div className="relative">
      {open && <MentionPopup rows={mentionRows} active={mentionIndex} onPick={onAcceptMention} />}
      {/* Mirror layer — paints the token chips under the transparent-text
          textarea. Its box metrics (padding/font/line-height) must track
          the Textarea's exactly or the chips drift off the glyphs. */}
      <div
        ref={mirrorRef}
        aria-hidden
        className="pointer-events-none select-none absolute inset-0 overflow-hidden whitespace-pre-wrap break-words px-2 pt-1 pb-2 text-[14px] leading-relaxed text-neutral-900"
      >
        {highlightComposerTokens(value)}
        {"\u200B"}
      </div>
      <Textarea
        ref={taRef}
        data-composer
        value={value}
        onChange={(e) => {
          onChange(e.target.value);
          onRefreshMention(e.target.value, e.target.selectionStart ?? e.target.value.length);
        }}
        onScroll={(e) => {
          if (mirrorRef.current) {
            mirrorRef.current.scrollTop = e.currentTarget.scrollTop;
          }
        }}
        onSelect={(e) => {
          const el = e.currentTarget;
          onRefreshMention(el.value, el.selectionStart ?? el.value.length);
        }}
        onKeyDown={(e) => {
          if (open) {
            if (e.key === "ArrowDown") {
              e.preventDefault();
              setMentionIndex(Math.min(mentionIndex + 1, mentionRows.length - 1));
              return;
            }
            if (e.key === "ArrowUp") {
              e.preventDefault();
              setMentionIndex(Math.max(mentionIndex - 1, 0));
              return;
            }
            if (e.key === "Enter" || e.key === "Tab") {
              e.preventDefault();
              const row = mentionRows[Math.min(mentionIndex, mentionRows.length - 1)];
              if (row) onAcceptMention(row);
              return;
            }
            if (e.key === "Escape") {
              e.preventDefault();
              onDismissMention();
              return;
            }
          }
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            onSubmit();
          }
        }}
        placeholder={
          streaming ? "插入指示引导生成 (Steer)…" : "输入消息… @ 引用文件 · / 命令 · $ 技能"
        }
        rows={1}
        className="relative min-h-[42px] max-h-[180px] resize-none border-0 shadow-none focus-visible:ring-0 px-2 pt-1 text-[14px] leading-relaxed bg-transparent text-transparent caret-neutral-800 selection:bg-[color-mix(in_srgb,var(--husk-n300)_70%,transparent)]"
      />
    </div>
  );
}

function ComposerToolbar({
  agentMode,
  agentModeLabel,
  switchAgentMode,
  modeLabel,
  mode,
  switchMode,
  activeModel,
  activeProvider,
  models,
  onSelectModel,
  hasReasoning,
  activeThinkingLevel,
  currentThinkingLabel,
  effectiveThinkingLevels,
  onSelectThinkingLevel,
  streaming,
  text,
  hasAttachments,
  onPick,
  submit,
  cancel,
}: {
  agentMode: string;
  agentModeLabel: string;
  switchAgentMode: (m: string) => Promise<void>;
  modeLabel: string;
  mode: string;
  switchMode: (m: string) => Promise<void>;
  activeModel: string;
  activeProvider: string;
  models: agent.ModelItem[];
  onSelectModel: (provider: string, model: string) => Promise<void>;
  hasReasoning: boolean;
  activeThinkingLevel: string;
  currentThinkingLabel: string;
  effectiveThinkingLevels: readonly { value: string; label: string; desc: string }[];
  onSelectThinkingLevel: (level: string) => Promise<void>;
  streaming: boolean;
  text: string;
  hasAttachments: boolean;
  onPick: (kind: "image" | "text" | "any") => void;
  submit: () => Promise<void>;
  cancel: () => Promise<void>;
}) {
  const canSubmit = text.trim().length > 0 || hasAttachments;

  return (
    <div className="flex items-center justify-between pt-1">
      <div className="flex items-center gap-1.5">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="添加附件"
              className="w-7 h-7 rounded-full text-neutral-500 hover:text-neutral-800 hover:bg-neutral-100 dark:text-[#9a9aa4] dark:hover:bg-[#2b2b32]"
            >
              <Plus className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" side="top" className="w-44">
            <DropdownMenuItem
              onClick={() => onPick("image")}
              className="flex items-center gap-2 text-xs py-2 cursor-pointer"
            >
              <Image className="h-3.5 w-3.5 text-neutral-500" />
              图片
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => onPick("text")}
              className="flex items-center gap-2 text-xs py-2 cursor-pointer"
            >
              <FileText className="h-3.5 w-3.5 text-neutral-500" />
              文本文件
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => onPick("any")}
              className="flex items-center gap-2 text-xs py-2 cursor-pointer"
            >
              <Paperclip className="h-3.5 w-3.5 text-neutral-500" />
              其他文件
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              aria-label="代理模式"
              className="flex items-center gap-1.5 px-2.5 py-1 text-[12px] font-medium text-neutral-600 bg-[color-mix(in_srgb,var(--husk-n100)_80%,transparent)] hover:bg-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] rounded-lg transition-colors border border-[color-mix(in_srgb,var(--husk-n200)_50%,transparent)] select-none cursor-pointer"
            >
              <Bot className="h-3.5 w-3.5 text-neutral-500" />
              <span>{agentModeLabel}</span>
              <ChevronsUpDown className="h-3.5 w-3.5 text-neutral-400 shrink-0" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" side="top" className="w-64">
            <DropdownMenuLabel className="text-xs text-neutral-500 font-normal">
              代理模式
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              {AGENT_MODES.map((m) => {
                const active = m.value === agentMode;
                return (
                  <DropdownMenuItem
                    key={m.value}
                    onClick={() => void switchAgentMode(m.value)}
                    className="flex flex-col items-start py-2 px-2 cursor-pointer rounded-lg gap-0.5"
                  >
                    <div className="flex items-center justify-between w-full">
                      <span className="font-medium text-xs text-neutral-800">
                        {m.label}
                      </span>
                      {active && <Check className="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400 shrink-0" />}
                    </div>
                    <span className="text-[11px] text-neutral-400 leading-tight">
                      {m.desc}
                    </span>
                  </DropdownMenuItem>
                );
              })}
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              aria-label="权限模式"
              className="flex items-center gap-1.5 px-2.5 py-1 text-[12px] font-medium text-neutral-600 bg-[color-mix(in_srgb,var(--husk-n100)_80%,transparent)] hover:bg-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] rounded-lg transition-colors border border-[color-mix(in_srgb,var(--husk-n200)_50%,transparent)] select-none cursor-pointer"
            >
              <ShieldCheck className="h-3.5 w-3.5 text-neutral-500" />
              <span>{modeLabel}</span>
              <ChevronsUpDown className="h-3.5 w-3.5 text-neutral-400 shrink-0" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" side="top" className="w-64">
            <DropdownMenuLabel className="text-xs text-neutral-500 font-normal">
              权限模式
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              {PERMISSION_MODES.map((m) => {
                const active = m.value === mode;
                return (
                  <DropdownMenuItem
                    key={m.value}
                    onClick={() => void switchMode(m.value)}
                    className="flex flex-col items-start py-2 px-2 cursor-pointer rounded-lg gap-0.5"
                  >
                    <div className="flex items-center justify-between w-full">
                      <span className="font-medium text-xs text-neutral-800">
                        {m.label}
                      </span>
                      {active && <Check className="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400 shrink-0" />}
                    </div>
                    <span className="text-[11px] text-neutral-400 leading-tight">
                      {m.desc}
                    </span>
                  </DropdownMenuItem>
                );
              })}
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>

        {hasReasoning && effectiveThinkingLevels.length > 0 && (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button
                aria-label={`思考推理强度: ${currentThinkingLabel}`}
                className="flex items-center gap-1.5 px-2.5 py-1 text-[12px] font-medium text-neutral-600 bg-[color-mix(in_srgb,var(--husk-n100)_80%,transparent)] hover:bg-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] rounded-lg transition-colors border border-[color-mix(in_srgb,var(--husk-n200)_50%,transparent)] select-none cursor-pointer"
              >
                <Brain
                  className={cn(
                    "h-3.5 w-3.5 shrink-0",
                    activeThinkingLevel === "off"
                      ? "text-neutral-400"
                      : "text-purple-600 dark:text-purple-400",
                  )}
                />
                <span className="truncate max-w-[90px]">
                  {activeThinkingLevel === "off" ? "思考关闭" : `思考: ${currentThinkingLabel}`}
                </span>
                <ChevronsUpDown className="h-3.5 w-3.5 text-neutral-400 shrink-0" />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" side="top" className="w-48">
              <DropdownMenuLabel className="text-xs text-neutral-500 font-normal">
                思考推理强度
              </DropdownMenuLabel>
              <DropdownMenuSeparator />
              <DropdownMenuGroup>
                {effectiveThinkingLevels.map((lvl) => {
                  const active = lvl.value === activeThinkingLevel;
                  return (
                    <DropdownMenuItem
                      key={lvl.value}
                      onClick={() => void onSelectThinkingLevel(lvl.value)}
                      className="flex items-center justify-between text-xs py-2 cursor-pointer"
                    >
                      <div className="flex flex-col gap-0.5 min-w-0 pr-2">
                        <span className="font-medium text-neutral-800">
                          {lvl.label}
                        </span>
                        <span className="text-[10.5px] text-neutral-400 truncate">
                          {lvl.desc}
                        </span>
                      </div>
                      {active && (
                        <Check className="h-3.5 w-3.5 text-purple-600 dark:text-purple-400 shrink-0" />
                      )}
                    </DropdownMenuItem>
                  );
                })}
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </div>

      <div className="flex items-center gap-2">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              aria-label={`当前模型: ${activeModel || "未选择"} (${activeProvider})`}
              className="flex items-center gap-1.5 px-2.5 py-1 text-[12px] font-medium text-neutral-600 hover:bg-neutral-100 rounded-lg transition-colors select-none cursor-pointer"
            >
              <span className="truncate max-w-[140px] font-mono text-[11.5px]">
                {activeModel || "选择模型"}
              </span>
              <ChevronsUpDown className="h-3.5 w-3.5 text-neutral-400 shrink-0" />
            </button>
          </DropdownMenuTrigger>

          <DropdownMenuContent align="end" side="top" className="w-56">
            <DropdownMenuLabel className="text-xs text-neutral-500 font-normal">
              模型
            </DropdownMenuLabel>
            <DropdownMenuSeparator />

            {models.length > 0 ? (
              <DropdownMenuGroup>
                {models.map((item) => {
                  const active = item.model === activeModel && item.provider === activeProvider;
                  return (
                    <DropdownMenuItem
                      key={`${item.provider}-${item.model}`}
                      onClick={() => void onSelectModel(item.provider, item.model)}
                      className="flex items-center justify-between text-xs py-2 cursor-pointer"
                    >
                      <div className="flex flex-col min-w-0 pr-2">
                        <span className="font-mono font-medium truncate text-neutral-900">
                          {item.model}
                        </span>
                        <span className="text-[10px] text-neutral-400">{item.provider}</span>
                      </div>
                      {active && <Check className="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400 shrink-0" />}
                    </DropdownMenuItem>
                  );
                })}
              </DropdownMenuGroup>
            ) : (
              <div className="px-3 py-3 text-xs text-neutral-400 text-center">未检测到模型</div>
            )}
          </DropdownMenuContent>
        </DropdownMenu>

        {streaming ? (
          <>
            {canSubmit && (
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    type="button"
                    className="w-8 h-8 rounded-full bg-neutral-100 hover:bg-neutral-200 text-neutral-700 border border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)] dark:bg-[#2b2b32] dark:hover:bg-[#34343d] dark:border-[#3f3f49] dark:text-[#d4d4d8] flex items-center justify-center cursor-pointer transition-all shadow-xs active:scale-95"
                    onClick={() => void submit()}
                    aria-label="排队发送"
                  >
                    <ListPlus className="h-4 w-4" />
                  </button>
                </TooltipTrigger>
                <TooltipContent side="top">排队发送 — 回合结束后自动发出 (Enter)</TooltipContent>
              </Tooltip>
            )}
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                className="group relative w-8 h-8 rounded-full bg-neutral-100 hover:bg-neutral-200 border border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)] dark:bg-[#2b2b32] dark:hover:bg-[#34343d] dark:border-[#3f3f49] flex items-center justify-center cursor-pointer transition-all shadow-xs"
                onClick={() => void cancel()}
                aria-label="中断回复"
              >
                <Orb
                  variant="B3"
                  size={16}
                  className="text-neutral-800 group-hover:opacity-0 transition-opacity duration-150"
                />
                <Square className="h-2.5 w-2.5 fill-neutral-800 text-neutral-800 absolute opacity-0 group-hover:opacity-100 transition-opacity duration-150" />
              </button>
            </TooltipTrigger>
            <TooltipContent side="top">中断当前回复</TooltipContent>
          </Tooltip>
          </>
        ) : canSubmit ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                className="w-8 h-8 rounded-full bg-neutral-900 hover:bg-black text-white flex items-center justify-center cursor-pointer transition-all shadow-xs active:scale-95"
                onClick={() => void submit()}
                aria-label="发送"
              >
                <ArrowUp className="h-4 w-4" />
              </button>
            </TooltipTrigger>
            <TooltipContent side="top">发送 (Enter)</TooltipContent>
          </Tooltip>
        ) : (
          <button
            type="button"
            disabled
            className="w-8 h-8 rounded-full bg-neutral-100 text-neutral-300 flex items-center justify-center cursor-not-allowed border border-[color-mix(in_srgb,var(--husk-n200)_50%,transparent)] dark:bg-[#26262d] dark:text-[#5b5b66] dark:border-[#3a3a44] transition-all select-none"
            aria-label="无法发送 (请输入内容)"
          >
            <ArrowUp className="h-4 w-4" />
          </button>
        )}
      </div>
    </div>
  );
}
