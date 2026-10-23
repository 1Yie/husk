/** The mention popup — absolutely positioned above the textarea, driven by
 * the `mention`/`mentionIndex` state the parent keeps in sync with the
 * caret. Rows arrive in one flat list; the section headers are drawn here, so
 * the keyboard index stays flat too. Mouse-down (not click) so the textarea
 * keeps focus. */

import { Fragment, useEffect, useRef } from "react";
import { FileCode, Plug, Sparkles, Terminal } from "@keyline-icons/react";
import { cn } from "@/lib/utils";
import { INK_MUTED, SURFACE_OVERLAY } from "@/lib/theme";

export type MentionGroup = "command" | "skill" | "plugin" | "file";

/** One selectable row — the composer builds the list, this file renders it. */
export interface MentionRow {
  key: string;
  label: string;
  hint: string;
  icon: "file" | "cmd" | "skill" | "plugin";
  /** What the pick writes into the composer. Absent on local rows: they run an
   *  action and never leave text behind. */
  insert?: string;
  group: MentionGroup;
  /** Argument placeholder for commands that take one. */
  args?: string;
  /** Handled by the composer UI — the row says so instead of pretending to
   *  insert a command. */
  local?: boolean;
  /** Bare command name, for rows backed by the builtin catalog. */
  command?: string;
}

const GROUP_LABEL: Record<MentionGroup, string> = {
  command: "命令",
  skill: "技能",
  plugin: "插件",
  file: "文件",
};

function RowIcon({ icon }: { icon: MentionRow["icon"] }) {
  if (icon === "file") return <FileCode className="h-3.5 w-3.5" />;
  if (icon === "skill") return <Sparkles className="h-3.5 w-3.5" />;
  if (icon === "plugin") return <Plug className="h-3.5 w-3.5" />;
  return <Terminal className="h-3.5 w-3.5" />;
}

export function MentionPopup({
  rows,
  active,
  onPick,
}: {
  rows: MentionRow[];
  active: number;
  onPick: (row: MentionRow) => void;
}) {
  const listRef = useRef<HTMLDivElement | null>(null);
  // Keep the active row visible on ↑/↓ — done manually instead of
  // scrollIntoView so ancestor scrollers (the chat stream) never move. The
  // headers shift the DOM index, so the rows are looked up by their own mark.
  useEffect(() => {
    const list = listRef.current;
    const row = list?.querySelectorAll<HTMLElement>("[data-mention-row]")[active];
    if (!list || !row) return;
    if (row.offsetTop < list.scrollTop) {
      list.scrollTop = row.offsetTop;
    } else if (row.offsetTop + row.offsetHeight > list.scrollTop + list.clientHeight) {
      list.scrollTop = row.offsetTop + row.offsetHeight - list.clientHeight;
    }
  }, [active]);

  if (rows.length === 0) {
    return (
      <div
        className={cn("absolute bottom-full left-2 right-2 mb-2 z-50 overflow-hidden", SURFACE_OVERLAY)}
        role="listbox"
      >
        <div className={cn("px-3 py-2.5 text-xs", INK_MUTED)}>无匹配项</div>
      </div>
    );
  }
  return (
    <div
      className={cn("absolute bottom-full left-2 right-2 mb-2 z-50 overflow-hidden", SURFACE_OVERLAY)}
      role="listbox"
    >
      <div ref={listRef} className="relative max-h-[18rem] overflow-y-auto py-1">
        {rows.map((r, i) => (
          <Fragment key={r.key}>
            {i === 0 || rows[i - 1].group !== r.group ? (
              <div
                className={cn(
                  "px-3 pb-1 text-[10.5px] font-medium tracking-wide text-neutral-400",
                  i === 0 ? "pt-1" : "pt-2",
                )}
              >
                {GROUP_LABEL[r.group]}
              </div>
            ) : null}
            <button
              type="button"
              role="option"
              aria-selected={i === active}
              data-mention-row
              onMouseDown={(e) => {
                e.preventDefault();
                onPick(r);
              }}
              className={cn(
                "w-full flex items-center gap-2 px-2.5 py-1.5 text-left text-[12.5px] transition-colors",
                i === active ? "bg-neutral-100" : "hover:bg-neutral-50",
              )}
            >
              <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md bg-[color-mix(in_srgb,var(--husk-black)_5%,transparent)] text-neutral-500">
                <RowIcon icon={r.icon} />
              </span>
              <span className="shrink-0 font-mono font-medium text-neutral-800">{r.label}</span>
              {r.args ? (
                <span className="shrink-0 font-mono text-[11px] text-neutral-400">{r.args}</span>
              ) : null}
              <span className="min-w-0 flex-1 truncate text-[11px] text-neutral-500">{r.hint}</span>
              {r.local ? (
                <span className={cn("shrink-0 text-[11px]", INK_MUTED)}>打开</span>
              ) : null}
            </button>
          </Fragment>
        ))}
      </div>
      <div
        className={cn(
          "flex items-center gap-1 border-t border-neutral-100 px-3 py-1.5 text-[11px] dark:border-[var(--husk-border)]",
          INK_MUTED,
        )}
      >
        ↑↓ 选择 · Enter 确认 · Esc 关闭
      </div>
    </div>
  );
}
