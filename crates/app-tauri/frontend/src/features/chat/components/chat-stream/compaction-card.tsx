// Inline card for a finished context-compaction pass. Both sources render
// through this one component — the live `UiEvent::Compacted` and the
// persisted `NoticeKind::Compacted` row replayed by `viewFromHistory` —
// so a reopened session shows the same accounting the live stream drew.
import { memo, useState } from "react";
import { Archive, ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";
import { fmtTokens } from "@/features/settings/usage";
import { MemoStreamdown } from "@/features/chat/components/chat-stream/markdown-stream";

export const CompactionCard = memo(function CompactionCard({
  before,
  after,
  removed,
  note,
}: {
  /** Context estimate before the pass, in tokens. */
  before: number;
  /** Context estimate after the splice. */
  after: number;
  /** Messages folded into the summary note. */
  removed: number;
  /** The compactor's summary — expanded on demand. */
  note: string;
}) {
  const [open, setOpen] = useState(false);
  const saved = before > 0 ? Math.round((1 - after / before) * 100) : 0;
  return (
    <div className="my-1 w-full min-w-0 overflow-hidden rounded-lg border border-[color-mix(in_srgb,var(--husk-n300)_75%,transparent)] bg-[color-mix(in_srgb,var(--husk-n100)_60%,transparent)]">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-[color-mix(in_srgb,var(--husk-n200)_45%,transparent)]"
      >
        <Archive className="h-3.5 w-3.5 shrink-0 text-neutral-500" />
        <span className="text-[12px] font-medium text-neutral-700">
          上下文已压缩
        </span>
        <span className="font-mono text-[11px] text-neutral-500 tabular-nums">
          {fmtTokens(before)} → {fmtTokens(after)}
        </span>
        {saved > 0 && (
          <span className="rounded-full bg-emerald-500/10 px-1.5 py-px font-mono text-[10px] text-emerald-600 tabular-nums">
            -{saved}%
          </span>
        )}
        {removed > 0 && (
          <span className="text-[11px] text-neutral-500">
            折叠 {removed} 条消息
          </span>
        )}
        <span className="ml-auto flex shrink-0 items-center gap-0.5 text-[11px] text-neutral-500">
          {open ? "收起摘要" : "查看摘要"}
          <ChevronDown
            className={cn("h-3.5 w-3.5 transition-transform", open && "rotate-180")}
          />
        </span>
      </button>
      {open && (
        <div className="border-t border-[color-mix(in_srgb,var(--husk-n300)_60%,transparent)] px-3 py-2">
          {note.trim() ? (
            <div className="select-text text-[13px] leading-relaxed text-neutral-600">
              <MemoStreamdown text={note} animating={false} />
            </div>
          ) : (
            <div className="text-[12px] text-neutral-400">
              本次压缩没有摘要文本
            </div>
          )}
        </div>
      )}
    </div>
  );
});
