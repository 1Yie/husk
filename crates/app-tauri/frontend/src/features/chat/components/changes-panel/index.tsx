// ChangesList — the 改动 tab body of the unified session dock: every file the
// agent touched, grouped by path, with each patch's diff stacked under it.
// Fed by the workspace's git change list, or — where the root is no repository
// — by `SessionView.changes` (applyEvent live, viewFromHistory on open); a
// workspace row carries one patch, a session row one per write.
// The dock owns the frame (title, tabs, close, width); this renders rows.

import { memo, useCallback, useMemo, useRef, useState } from "react";
import { ChevronDown, ChevronRight, FilePlus2, FileDiff, FileX2 } from "lucide-react";
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

/** Memoized on the entry itself: `recordFileChanges` replaces only the entry
 *  it touched, so a new write re-renders that file's row and nothing else. */
const FileRow = memo(function FileRow({
  change,
  open,
  onToggle,
}: {
  change: FileChange;
  open: boolean;
  onToggle: (path: string) => void;
}) {
  const { dir, base } = splitPath(change.path);
  return (
    <div className="w-full">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => onToggle(change.path)}
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
        <span className="min-w-0 flex-1 truncate text-[12px] text-neutral-700">
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
              <DiffView diff={p.diff} file={change.path} maxHeight={340} />
            </div>
          ))}
          {/* A git row can have no patch to draw: git keeps no line diff for a
              binary file, and an oversized one is dropped rather than rendered. */}
          {change.patches.length === 0 && change.binary && (
            <div className="px-1 text-[10.5px] text-neutral-400">二进制文件</div>
          )}
          {change.patches.length === 0 && !change.binary && change.truncated && (
            <div className="px-1 text-[10.5px] text-neutral-400">文件过大，未显示差异</div>
          )}
          {change.fuzzy && (
            <div className="px-1 text-[10.5px] text-amber-600 dark:text-amber-400">
              部分补丁为近似匹配，请核对
            </div>
          )}
        </div>
      )}
    </div>
  );
});

/** The 改动 view's body — file rows with their patches. The dock owns the
 *  frame (title, tabs, scrolling), so this renders rows only. Memoized: the
 *  page re-renders on every streamed token, and `changes` keeps its identity
 *  until a write lands. */
export const ChangesList = memo(function ChangesList({ changes }: { changes: FileChange[] }) {
  // Files start expanded — a first open answers "what changed" directly —
  // but a long agentic turn can produce dozens of patches. Mounting every
  // DiffView for all of them put thousands of nodes in the dock (the dock
  // auto-opens on the first change), so above a small threshold rows start
  // collapsed and expand on click.
  const [closed, setClosed] = useState<Set<string>>(new Set());
  const [opened, setOpened] = useState<Set<string>>(new Set());
  const total = useMemo(
    () => ({
      adds: changes.reduce((n, c) => n + c.adds, 0),
      dels: changes.reduce((n, c) => n + c.dels, 0),
      patches: changes.reduce((n, c) => n + c.patches.length, 0),
    }),
    [changes],
  );
  const crossed = changes.length > 4 || total.patches > 8;
  // Sticky and one-way: the threshold is crossed mid-turn as changes stream
  // in, and flipping the whole list to collapsed at that instant folded away
  // the diff the reader was looking at. On the crossing only the rows that
  // were ALREADY open stay open (they are mounted either way); files that
  // arrive in the same flush or later come in collapsed, which is where the
  // node budget actually went. Sticky also keeps a shrinking list from
  // flip-flopping back to expanded and remounting every DiffView.
  const prevPathsRef = useRef<string[]>([]);
  const [dense, setDense] = useState(crossed);
  if (crossed && !dense) {
    setDense(true);
    setOpened(new Set(prevPathsRef.current));
    setClosed(new Set());
  }
  prevPathsRef.current = changes.map((c) => c.path);
  const toggle = useCallback(
    (path: string) => {
      const flip = (prev: Set<string>) => {
        const next = new Set(prev);
        if (next.has(path)) next.delete(path);
        else next.add(path);
        return next;
      };
      if (dense) setOpened(flip);
      else setClosed(flip);
    },
    [dense],
  );

  return (
    <div className="w-full">
      {changes.map((c) => (
        <FileRow
          key={c.path}
          change={c}
          open={dense ? opened.has(c.path) : !closed.has(c.path)}
          onToggle={toggle}
        />
      ))}
    </div>
  );
});
