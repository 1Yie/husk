/** The mention popup — absolutely positioned above the textarea, driven by
 * the `mention`/`mentionIndex` state the parent keeps in sync with the
 * caret. Mouse-down (not click) so the textarea keeps focus. */

import { useEffect, useRef } from "react";
import { FileCode, Sparkles, Terminal } from "@keyline-icons/react";
import { cn } from "@/lib/utils";
import { INK_MUTED, SURFACE_OVERLAY } from "@/lib/theme";

export function MentionPopup({
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
      <div className={cn("absolute bottom-full left-2 right-2 mb-2 z-50 px-3 py-2.5 text-xs", INK_MUTED)}>
        无匹配项
      </div>
    );
  }
  return (
    <div
      className={cn("absolute bottom-full left-2 right-2 mb-2 z-50 overflow-hidden", SURFACE_OVERLAY)}
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
            <span className="shrink-0 text-neutral-500">
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
            <span className="truncate text-[11px] text-neutral-500">
              {r.hint}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}
