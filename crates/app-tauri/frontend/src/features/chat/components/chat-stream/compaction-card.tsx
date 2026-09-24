// Inline card for a context-compaction pass. Both sources render through
// this one component — the live `UiEvent::Compacted` and the persisted
// `NoticeKind::Compacted` row replayed by `viewFromHistory` — so a reopened
// session shows the same accounting the live stream drew. While the pass is
// running (`pending`) the same card is a progress placeholder: it appears at
// the moment the phase starts and is settled in place when the event lands.
import { memo, useState } from "react";
import { Archive, ChevronDown, Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { fmtTokens } from "@/features/settings/usage";
import { MemoStreamdown } from "@/features/chat/components/chat-stream/markdown-stream";

export const CompactionCard = memo(function CompactionCard({
  before,
  after,
  removed,
  note,
  pending = false,
}: {
  /** Context estimate before the pass, in tokens. */
  before: number;
  /** Context estimate after the splice. */
  after: number;
  /** Messages folded into the summary note. */
  removed: number;
  /** The compactor's summary — expanded on demand. */
  note: string;
  /** The pass is still running — render the progress placeholder. */
  pending?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const saved = before > 0 ? Math.round((1 - after / before) * 100) : 0;
  const canExpand = !pending && note.trim().length > 0;
  const expanded = canExpand && open;
  return (
    <div
      className={cn(
        "my-1 w-full min-w-0 overflow-hidden rounded-lg border bg-[color-mix(in_srgb,var(--husk-n100)_60%,transparent)]",
        pending
          ? "border-[color-mix(in_srgb,var(--husk-n300)_55%,transparent)]"
          : "border-[color-mix(in_srgb,var(--husk-n300)_75%,transparent)]"
      )}
    >
      <button
        type="button"
        onClick={() => canExpand && setOpen((o) => !o)}
        aria-expanded={canExpand ? expanded : undefined}
        disabled={!canExpand}
        className={cn(
          "flex w-full items-center gap-2 px-3 py-2 text-left transition-colors",
          canExpand &&
            "hover:bg-[color-mix(in_srgb,var(--husk-n200)_45%,transparent)]"
        )}
      >
        {pending ? (
          <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-neutral-500" />
        ) : (
          <Archive className="h-3.5 w-3.5 shrink-0 text-neutral-500" />
        )}
        <span className="text-[12px] font-medium text-neutral-700">
          {pending ? "正在压缩上下文…" : "上下文已压缩"}
        </span>
        {!pending && (
          <>
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
          </>
        )}
        {canExpand && (
          <span className="ml-auto flex shrink-0 items-center gap-0.5 text-[11px] text-neutral-500">
            {expanded ? "收起摘要" : "查看摘要"}
            <ChevronDown
              className={cn(
                "h-3.5 w-3.5 transition-transform",
                expanded && "rotate-180"
              )}
            />
          </span>
        )}
      </button>
      {expanded && (
        <div className="border-t border-[color-mix(in_srgb,var(--husk-n300)_60%,transparent)] px-3 py-2">
          <div className="select-text text-[13px] leading-relaxed text-neutral-600">
            <MemoStreamdown text={note} animating={false} />
          </div>
        </div>
      )}
    </div>
  );
});
