// ChatStream — matches gensei's web ConversationThread design 1:1.
// Replaces legacy app-desktop timeline dots, lines, and avatar cards.

import { useEffect, useRef } from "react";
import { cjk } from "@streamdown/cjk";
import { Streamdown } from "streamdown";
import type { SessionView, StreamItem } from "../../hooks/useAgent";
import { AssistantStatus } from "../assistant-status";
import { ToolChips, type ToolChipRow } from "../tool-chips";
import { Check, Copy } from "@keyline-icons/react";
import { codexMarkdownComponents } from "./markdown-components";
import { prismCodePlugin } from "../../lib/syntax-highlight";

const streamdownIcons = {
  CheckIcon: Check,
  CopyIcon: Copy,
};

/** The kernel expands `@path` mentions into `` `<workspace-file …>` `` +
 * fenced-content blocks, and `/skill`/`$skill` into a skill-prompt
 * preamble — that text is for the model. The user bubble shows only the
 * compact `@path` / `/name` chip. */
const WORKSPACE_FILE_BLOCK =
  /\s*`<workspace-file path="([^"]+)">`\s*```\n[\s\S]*?```\s*/g;
const SKILL_PROMPT_RE =
  /^The user invoked the `([/$][^\s`]+)` skill\. Follow its instructions exactly\.\n\n---\n[\s\S]*$/;

function collapsePromptArtifacts(md: string): string {
  const skill = SKILL_PROMPT_RE.exec(md);
  if (skill) {
    const args = /Skill arguments: ([\s\S]+)$/.exec(md)?.[1]?.trim();
    return `\`${skill[1]}${args ? ` ${args}` : ""}\``;
  }
  return md
    .replace(WORKSPACE_FILE_BLOCK, (_m, p) => `\`@${p}\` `)
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}


interface Props {
  view: SessionView;
}

type AssistantStep =
  | { type: "thinking"; text: string; done: boolean }
  | { type: "text"; text: string; streaming: boolean }
  | { type: "tools"; rows: ToolChipRow[] }
  | { type: "system"; text: string };

interface Turn {
  id: string;
  userText?: string;
  steps: AssistantStep[];
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
              status: item.content === undefined ? "running" : item.ok === false ? "aborted" : "done",
              detail: item.content ? [item.content] : item.approval?.diff ? [item.approval.diff] : undefined,
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

export function ChatStream({ view }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const endRef = useRef<HTMLDivElement>(null);
  const pinnedRef = useRef(true);

  useEffect(() => {
    if (pinnedRef.current) {
      endRef.current?.scrollIntoView({ block: "end" });
    }
  }, [view.items]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    pinnedRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
  };

  const turns = parseTurns(view.items);

  const hasActiveItem = view.items.some((i) => {
    if (i.kind === "thinking" && !i.done) return true;
    if (i.kind === "assistant" && i.streaming) return true;
    if (i.kind === "tool" && i.content === undefined) return true;
    if (i.kind === "approval" && !i.resolved) return true;
    return false;
  });

  const showReplyWait = view.streaming && !hasActiveItem;

  return (
    <div
      className="stream-scroll bg-white flex-1 overflow-y-auto w-full select-text"
      ref={scrollRef}
      onScroll={onScroll}
    >
      <div className="max-w-3xl w-full mx-auto px-4 pt-6 pb-32 flex flex-col gap-6 min-h-full">
        {view.items.length === 0 && !view.streaming && (
          <div className="my-auto text-neutral-400 text-sm text-center select-none py-16">
            开始新对话 — 在下方输入框发送消息。
          </div>
        )}

        {turns.map((turn, turnIdx) => (
          <div key={turn.id} className="flex w-full flex-col gap-6">
            {turn.userText && (
              <div className="bg-neutral-100 dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 ms-auto flex w-fit max-w-[80%] flex-col gap-2 rounded-xl px-3.5 py-2.5 text-[14px]">
                <div className="min-w-0 text-[14px] leading-relaxed [&_p]:max-w-none [&_p:first-child]:mt-0 [&_p:last-child]:mb-0 select-text">
                  <Streamdown
                    plugins={{ cjk, code: prismCodePlugin as any }}
                    shikiTheme={["github-dark", "github-dark"]}
                    components={codexMarkdownComponents}
                    icons={streamdownIcons}
                  >
                    {collapsePromptArtifacts(turn.userText)}
                  </Streamdown>
                </div>
              </div>
            )}

            {turn.steps.length > 0 && (
              <div className="flex w-full flex-col items-start gap-4">
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
                    const isLastTurn = turnIdx === turns.length - 1;
                    const isLastStep = stepIdx === turn.steps.length - 1;
                    const isAnimating = view.streaming && isLastTurn && isLastStep;

                    return (
                      <div
                        className="w-full min-w-0 text-[14px] text-neutral-900 dark:text-neutral-100 [&_p]:max-w-none"
                        key={`step-${stepIdx}`}
                      >
                        <Streamdown
                          isAnimating={isAnimating}
                          plugins={{ cjk, code: prismCodePlugin as any }}
                          shikiTheme={["github-dark", "github-dark"]}
                          components={codexMarkdownComponents}
                          icons={streamdownIcons}
                        >
                          {step.text}
                        </Streamdown>
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

        {showReplyWait && <AssistantStatus mode="reply" thinkingText="" />}
        {/* Extra breathing room above the composer — the last message
         * shouldn't sit flush against the floating input card. */}
        <div className="h-8 flex-none" />
        <div ref={endRef} />
      </div>
    </div>
  );
}
