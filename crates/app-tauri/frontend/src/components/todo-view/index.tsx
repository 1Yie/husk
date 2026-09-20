// TodoView — Clean, structured UI presentation for agent task/todo tracking.
// Parses `todo` tool output and renders an interactive-looking, progress-tracked checklist.

import { useMemo } from "react";
import { Check, ListCheck } from "@keyline-icons/react";
import { cn } from "../../lib/utils";

interface Props {
  content: string;
}

export interface TodoItem {
  id: number;
  text: string;
  done: boolean;
}

export function parseTodos(raw: string): {
  items: TodoItem[];
  doneCount: number;
  totalCount: number;
  headerMessage?: string;
} {
  const items: TodoItem[] = [];
  const lines = raw.split("\n");
  let headerMessage: string | undefined;

  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed) continue;

    // Check for [ ] #id text or [x] #id text
    const taskMatch = trimmed.match(/^\[([ xX])\]\s*#?(\d+)\s+(.+)$/);
    if (taskMatch) {
      const done = taskMatch[1].toLowerCase() === "x";
      const id = parseInt(taskMatch[2], 10);
      const text = taskMatch[3].trim();
      items.push({ id, text, done });
      continue;
    }

    // Capture leading messages like "Added #1 ...", "Removed #2 ...", "Cleared ... todos"
    if (
      trimmed.startsWith("Added #") ||
      trimmed.startsWith("Removed #") ||
      trimmed.startsWith("Cleared ") ||
      trimmed.endsWith("done")
    ) {
      if (!trimmed.endsWith("done")) {
        headerMessage = trimmed;
      }
    }
  }

  const doneCount = items.filter((t) => t.done).length;
  const totalCount = items.length;

  return { items, doneCount, totalCount, headerMessage };
}

export function TodoView({ content }: Props) {
  const { items, doneCount, totalCount, headerMessage } = useMemo(
    () => parseTodos(content),
    [content]
  );

  if (items.length === 0) {
    return (
      <div className="w-full rounded-xl border border-neutral-200 dark:border-neutral-800 bg-[#fafafa] dark:bg-[#121214] p-3 text-xs text-neutral-500 font-mono">
        {content || "暂无任务项"}
      </div>
    );
  }

  const percentage = totalCount > 0 ? Math.round((doneCount / totalCount) * 100) : 0;

  return (
    <div className="w-full rounded-xl border border-neutral-200 dark:border-neutral-800 bg-[#fafafa] dark:bg-[#141416] p-3 shadow-2xs select-text">
      <div className="flex items-center justify-between gap-3 pb-2.5 mb-2 border-b border-neutral-200/70 dark:border-neutral-800/80 select-none">
        <div className="flex items-center gap-2 min-w-0">
          <ListCheck className="h-4 w-4 text-emerald-600 dark:text-emerald-400 shrink-0" />
          <span className="text-xs font-semibold text-neutral-800 dark:text-neutral-200 shrink-0">
            任务清单
          </span>
          {headerMessage && (
            <span className="text-[11px] text-neutral-400 font-normal truncate">
              · {headerMessage}
            </span>
          )}
        </div>
        <div className="flex items-center gap-2.5 shrink-0">
          <div className="w-24 h-1.5 rounded-full bg-neutral-200 dark:bg-neutral-800 overflow-hidden">
            <div
              className="h-full bg-emerald-500 transition-all duration-300 rounded-full"
              style={{ width: `${percentage}%` }}
            />
          </div>
          <span className="text-[11px] font-mono font-medium text-neutral-500 dark:text-neutral-400">
            {doneCount}/{totalCount} ({percentage}%)
          </span>
        </div>
      </div>

      <div className="flex flex-col gap-1">
        {items.map((item) => (
          <div
            key={item.id}
            className={cn(
              "group flex items-start gap-2.5 py-1 px-1.5 rounded-lg transition-colors",
              item.done
                ? "text-neutral-400 dark:text-neutral-500"
                : "text-neutral-800 dark:text-neutral-200 hover:bg-neutral-100/60 dark:hover:bg-neutral-800/40"
            )}
          >
            <span className="flex h-5 w-4 items-center justify-center shrink-0">
              {item.done ? (
                <span className="inline-flex items-center justify-center w-3.5 h-3.5 rounded-full bg-emerald-500/20 text-emerald-600 dark:text-emerald-400">
                  <Check className="h-2.5 w-2.5 stroke-[2.8]" />
                </span>
              ) : (
                <span className="inline-flex items-center justify-center w-3.5 h-3.5 rounded-full border border-neutral-300 dark:border-neutral-600 group-hover:border-neutral-400 dark:group-hover:border-neutral-500 transition-colors" />
              )}
            </span>
            <span className="flex h-5 items-center font-mono text-[11px] text-neutral-400 dark:text-neutral-500 shrink-0 select-none">
              #{item.id}
            </span>
            <span
              className={cn(
                "text-[13px] leading-5 break-words flex-1",
                item.done ? "line-through opacity-75" : "font-normal"
              )}
            >
              {item.text}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
