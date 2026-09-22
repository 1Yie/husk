// Raw history viewer — opens from the title-bar chart icon and shows the session as
// persisted: role, content, tool_calls, tool results, notices, timestamps. Rows are
// collapsed by default; JSON is highlighted lazily.
import { memo, useMemo, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Copy, Check, ChevronRight } from "@keyline-icons/react";
import { highlightCodeToHtml } from "@/lib/syntax-highlight";
import { cn } from "@/lib/utils";
import type { ChatMessage } from "../../types";

const ROLE_STYLE: Record<string, string> = {
  user: "text-blue-600 dark:text-blue-400 bg-blue-500/10",
  assistant: "text-emerald-600 dark:text-emerald-400 bg-emerald-500/10",
  tool: "text-amber-600 dark:text-amber-400 bg-amber-500/10",
  system: "text-neutral-500 bg-neutral-500/10",
};

function preview(m: ChatMessage): string {
  if (m.tool_calls?.length) {
    return m.tool_calls.map((t) => t.function.name).join(", ");
  }
  const c = (m.content ?? "").replace(/\s+/g, " ").trim();
  return c.length > 90 ? c.slice(0, 90) + "…" : c;
}

const MessageRow = memo(function MessageRow({
  m,
  index,
}: {
  m: ChatMessage;
  index: number;
}) {
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState(false);
  // Stringify only when expanded — keeps the initial list cheap.
  const json = useMemo(
    () => (open ? JSON.stringify(m, null, 2) : ""),
    [open, m],
  );
  const html = useMemo(
    () => (open ? highlightCodeToHtml(json, "json") : ""),
    [open, json],
  );

  const copyOne = async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await navigator.clipboard.writeText(JSON.stringify(m, null, 2));
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {}
  };

  return (
    <div className="border-b border-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] last:border-b-0">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-3 py-1.5 text-left hover:bg-[color-mix(in_srgb,var(--husk-n100)_70%,transparent)] transition-colors"
      >
        <ChevronRight
          className={cn(
            "h-3 w-3 flex-none text-neutral-500 transition-transform",
            open && "rotate-90",
          )}
        />
        <span className="w-10 flex-none font-mono text-[10px] text-neutral-500 tabular-nums">
          #{index}
        </span>
        <span
          className={cn(
            "flex-none rounded px-1.5 py-px font-mono text-[10px] font-medium",
            ROLE_STYLE[m.role] ?? ROLE_STYLE.system,
          )}
        >
          {m.role}
        </span>
        {m.tool_call_id && (
          <span className="flex-none font-mono text-[10px] text-neutral-500">
            →{m.tool_call_id.slice(0, 12)}
          </span>
        )}
        <span className="min-w-0 flex-1 truncate text-[11.5px] text-neutral-500">
          {preview(m)}
        </span>
        <span
          role="button"
          tabIndex={-1}
          onClick={copyOne}
          className="flex-none rounded p-1 text-neutral-500 hover:text-neutral-700 hover:bg-neutral-200/60"
          title="复制此条 JSON"
        >
          {copied ? (
            <Check className="h-3 w-3 text-emerald-500" />
          ) : (
            <Copy className="h-3 w-3" />
          )}
        </span>
      </button>
      {open && (
        <div className="mx-3 mb-2 overflow-auto rounded-lg border border-neutral-200 bg-neutral-50 max-h-[320px]">
          <pre
            className="p-3 font-mono text-[11px] leading-relaxed text-neutral-700 whitespace-pre-wrap break-all"
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
  messages,
  loading,
  title,
}: {
  open: boolean;
  onOpenChange: (v: boolean) => void;
  messages: ChatMessage[] | null;
  loading: boolean;
  title?: string;
}) {
  const [copiedAll, setCopiedAll] = useState(false);

  const copyAll = async () => {
    if (!messages) return;
    try {
      await navigator.clipboard.writeText(JSON.stringify(messages, null, 2));
      setCopiedAll(true);
      setTimeout(() => setCopiedAll(false), 1500);
    } catch {}
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl h-[82vh] flex flex-col gap-0 p-0 overflow-hidden">
        <DialogHeader className="flex-none px-4 pt-4 pb-3 border-b border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)]">
          <div className="flex items-center justify-between pr-8">
            <div className="flex items-baseline gap-2 min-w-0">
              <DialogTitle className="text-[13px] font-semibold text-neutral-800 flex-none">
                原始对话记录
              </DialogTitle>
              {messages && (
                <span className="flex-none rounded-full bg-neutral-100 dark:bg-neutral-800 px-1.5 py-px font-mono text-[10px] leading-[14px] text-neutral-500 tabular-nums">
                  {messages.length}
                </span>
              )}
              {title && (
                <span className="min-w-0 truncate text-[11px] text-neutral-500">
                  {title}
                </span>
              )}
            </div>
            <button
              type="button"
              onClick={copyAll}
              disabled={!messages}
              className="flex flex-none items-center gap-1.5 rounded-md px-2 py-1 text-[11px] text-neutral-500 hover:text-neutral-800 hover:bg-neutral-100 transition-colors disabled:opacity-40"
            >
              {copiedAll ? (
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
            </button>
          </div>
        </DialogHeader>

        <div className="flex-1 min-h-0 overflow-y-auto">
          {!open ? (
            // Closing: Radix keeps the tree mounted through the exit
            // animation — unmount the heavy row list NOW so the zoom-out
            // animates an empty shell instead of compositing hundreds of
            // rows (that repaint is the visible flash on WebKitGTK).
            <div className="h-full" />
          ) : loading ? (
            <div className="flex h-full items-center justify-center text-[12px] text-neutral-500">
              正在读取…
            </div>
          ) : !messages || messages.length === 0 ? (
            <div className="flex h-full items-center justify-center text-[12px] text-neutral-500">
              暂无记录
            </div>
          ) : (
            messages.map((m, i) => <MessageRow key={i} m={m} index={i} />)
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
