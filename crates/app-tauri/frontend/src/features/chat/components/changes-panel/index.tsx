// ChangesPanel — the right-side dock: every file the agent touched,
// grouped by path, with each patch's diff stacked under it. Fed by
// `SessionView.changes` (applyEvent live, viewFromHistory on open).

import { useMemo, useState } from "react";
import { ChevronDown, ChevronRight, FilePlus2, FileDiff, FileX2, X } from "lucide-react";
import { DiffView } from "@/features/chat/components/diff-view";
import { cn } from "@/lib/utils";
import type { FileChange } from "@/features/chat/hooks/stream-view";

function KindIcon({ kind }: { kind: FileChange["kind"] }) {
  const cls = "h-3.5 w-3.5 flex-none";
  switch (kind) {
    case "add":
      return <FilePlus2 className={cn(cls, "text-emerald-600 dark:text-emerald-400")} />;
    case "delete":
      return <FileX2 className={cn(cls, "text-rose-600 dark:text-rose-400")} />;
    default:
      return <FileDiff className={cn(cls, "text-amber-600 dark:text-amber-400")} />;
  }
}

/** `path/to/file.rs` → (`path/to`, `file.rs`) — the row shows the file
 * name bold and its directory muted, same convention as editor tabs. */
function splitPath(p: string): { dir: string; base: string } {
  const i = p.lastIndexOf("/");
  if (i < 0) return { dir: "", base: p };
  return { dir: p.slice(0, i + 1), base: p.slice(i + 1) };
}

function FileRow({
  change,
  open,
  onToggle,
}: {
  change: FileChange;
  open: boolean;
  onToggle: () => void;
}) {
  const { dir, base } = splitPath(change.path);
  return (
    <div className="w-full">
      <button
        type="button"
        aria-expanded={open}
        onClick={onToggle}
        className="group flex w-full items-center gap-1.5 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-[color-mix(in_srgb,var(--husk-n200)_45%,transparent)]"
      >
        <span className="flex-none text-neutral-500">
          {open ? (
            <ChevronDown className="h-3 w-3" />
          ) : (
            <ChevronRight className="h-3 w-3" />
          )}
        </span>
        <KindIcon kind={change.kind} />
        <span className="min-w-0 flex-1 truncate text-[12px] text-neutral-700 dark:text-neutral-300">
          <span className="font-medium">{base}</span>
          {dir && <span className="ml-1 text-[10.5px] text-neutral-400">{dir}</span>}
        </span>
        {change.pending && (
          <span className="flex-none rounded bg-amber-500/15 px-1 py-px text-[9.5px] font-medium text-amber-600 dark:text-amber-400">
            待批准
          </span>
        )}
        {change.patches.length > 1 && (
          <span className="flex-none text-[10px] text-neutral-400">
            ×{change.patches.length}
          </span>
        )}
        <span className="flex-none font-mono text-[10.5px] tabular-nums">
          {change.adds > 0 && (
            <span className="text-emerald-600 dark:text-emerald-400">+{change.adds}</span>
          )}
          {change.dels > 0 && (
            <span className="ml-1 text-rose-600 dark:text-rose-400">-{change.dels}</span>
          )}
        </span>
      </button>
      {/* Patch cards span the full row width — the panel is too narrow
          for a filename-alignment gutter. */}
      {open && (
        <div className="mb-1.5 flex flex-col gap-1.5">
          {change.patches.map((p) => (
            <div key={p.n} className="min-w-0">
              {change.patches.length > 1 && (
                <div className="mb-0.5 px-1 text-[10px] font-medium text-neutral-400">
                  {p.tool} · 第 {p.n} 次修改
                </div>
              )}
              <DiffView diff={p.diff} maxHeight={340} />
            </div>
          ))}
          {change.fuzzy && (
            <div className="px-1 text-[10.5px] text-amber-600 dark:text-amber-400">
              部分补丁为近似匹配，请核对
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export function ChangesPanel({
  changes,
  onClose,
}: {
  changes: FileChange[];
  onClose?: () => void;
}) {
  // Files start expanded — a first open answers "what changed" directly.
  const [closed, setClosed] = useState<Set<string>>(new Set());
  const total = useMemo(
    () => ({
      adds: changes.reduce((n, c) => n + c.adds, 0),
      dels: changes.reduce((n, c) => n + c.dels, 0),
    }),
    [changes],
  );

  return (
    <aside className="flex h-full w-[300px] flex-none flex-col overflow-hidden border-l border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)] bg-[color-mix(in_srgb,var(--husk-n50)_70%,transparent)] select-none">
      <div className="flex h-9 w-[300px] flex-none items-center gap-2 border-b border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)] px-3">
        <span className="text-[12px] font-medium text-neutral-700 dark:text-neutral-300">
          改动
        </span>
        <span className="text-[10.5px] text-neutral-400">
          {changes.length} 个文件
        </span>
        <span className="font-mono text-[10.5px] tabular-nums">
          {total.adds > 0 && (
            <span className="text-emerald-600 dark:text-emerald-400">+{total.adds}</span>
          )}
          {total.dels > 0 && (
            <span className="ml-1 text-rose-600 dark:text-rose-400">-{total.dels}</span>
          )}
        </span>
        <span className="flex-1" />
        {onClose && (
          <button
            type="button"
            onClick={onClose}
            aria-label="关闭改动面板"
            className="flex-none rounded p-1 text-neutral-500 transition-colors hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] hover:text-neutral-700"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        )}
      </div>

      {/* No overflow-x-hidden — it would clip each DiffView's own
          horizontal scroll. Long paths truncate at the row instead. */}
      <div className="w-[300px] flex-1 min-h-0 overflow-y-auto px-1.5 py-1.5">
        {changes.length === 0 ? (
          <div className="px-3 py-8 text-center text-[12px] text-neutral-400">
            暂无文件改动
          </div>
        ) : (
          changes.map((c) => (
            <FileRow
              key={c.path}
              change={c}
              open={!closed.has(c.path)}
              onToggle={() =>
                setClosed((prev) => {
                  const next = new Set(prev);
                  if (next.has(c.path)) next.delete(c.path);
                  else next.add(c.path);
                  return next;
                })
              }
            />
          ))
        )}
      </div>
    </aside>
  );
}
