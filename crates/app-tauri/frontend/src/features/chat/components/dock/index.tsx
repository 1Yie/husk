// SessionDock — the one right-hand sidebar. Everything a session produces
// hangs off it: the files the agent changed (改动), the files it generated
// (产物), the background jobs it started (后台) and the subagents it delegated
// (子代理), as tabs. It is independent of the workspace mode (编程 and 工作 both
// get the same dock) and owns its own UI state — open, active tab, width —
// persisted across sessions.
//
// The frame is the dock's; the tab bodies are `ChangesList` / `ArtifactsList`
// / `JobsList` / `SubagentsList`.

import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { Bot, FileDiff, Package, SquareTerminal, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { ChangesList } from "@/features/chat/components/changes-panel";
import { ArtifactsList } from "@/features/chat/components/artifacts-panel";
import { JobsList } from "@/features/chat/components/jobs-panel";
import { SubagentsList } from "@/features/chat/components/subagents-panel";
import type { Artifact, BgJob, FileChange } from "@/features/chat/hooks/stream-view";
import type { SessionChild } from "@/features/chat/hooks/running-children";
import { DOCK_MIN_WIDTH, clampDockWidth, useDockStore, type DockTab } from "./state";

const TAB_META: Record<DockTab, { Icon: typeof FileDiff; label: string; unit: string }> = {
  changes: { Icon: FileDiff, label: "改动", unit: "个文件" },
  artifacts: { Icon: Package, label: "产物", unit: "个文件" },
  jobs: { Icon: SquareTerminal, label: "后台", unit: "个任务" },
  subagents: { Icon: Bot, label: "子代理", unit: "个" },
};

function TabButton({
  tab,
  active,
  files,
  onSelect,
}: {
  tab: DockTab;
  active: boolean;
  files: number;
  onSelect: (tab: DockTab) => void;
}) {
  const { Icon, label, unit } = TAB_META[tab];
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      aria-label={`${label}（${files} ${unit}）`}
      onClick={() => onSelect(tab)}
      className={cn(
        // The session sidebar's mode switch in miniature: a recessed track,
        // the active tab a raised white capsule on it.
        "relative flex h-6 flex-none items-center gap-1 rounded-full px-2.5 text-[12px] transition-all duration-150",
        active
          ? "bg-white font-medium text-neutral-800 shadow-[0_1px_2px_rgba(0,0,0,0.08)] dark:bg-[var(--husk-hover)] dark:shadow-[0_1px_2px_rgba(0,0,0,0.6)]"
          : "text-neutral-500 hover:text-neutral-800",
      )}
    >
      <Icon className="h-3.5 w-3.5 flex-none" />
      <span>{label}</span>
      <span className="font-mono text-[10.5px] tabular-nums text-neutral-400">{files}</span>
    </button>
  );
}

export function SessionDock({
  changes,
  artifacts,
  jobs = [],
  jobsAt = 0,
  subagents = [],
  root = "",
  session = 0,
  onJobs,
  open: openProp,
  snap = false,
  onClose,
}: {
  changes: FileChange[];
  artifacts: Artifact[];
  /** The session's background jobs, and when that snapshot arrived. */
  jobs?: BgJob[];
  jobsAt?: number;
  /** Every delegated child, finished ones included — their cards, and a
   *  steer box on the ones still out. */
  subagents?: SessionChild[];
  /** Which session the 后台 controls address. */
  root?: string;
  session?: number;
  /** A control call's returned list — written back to the session's view. */
  onJobs?: (jobs: BgJob[]) => void;
  /** Whether the sidebar is shown. Defaults to the store flag; the page
   *  additionally ANDs in "has content" so an empty dock never occupies the
   *  column (a fresh session inherits the previous one's open flag). */
  open?: boolean;
  /** Apply open/close instantly, without the slide — set by the page for the
   *  commit that follows a session switch, which is a jump, not a toggle. */
  snap?: boolean;
  /** A user close — the page uses it to suppress this session's auto-open. */
  onClose?: () => void;
}) {
  const storeOpen = useDockStore((s) => s.open);
  const open = openProp ?? storeOpen;
  const tab = useDockStore((s) => s.tab);
  const width = useDockStore((s) => s.width);
  const setOpen = useDockStore((s) => s.setOpen);
  const setTab = useDockStore((s) => s.setTab);
  const setWidth = useDockStore((s) => s.setWidth);

  // Width is dragged; a CSS transition would fight the pointer, so the slide
  // is switched off for the duration of the drag. The live width stays local
  // and lands in the store (and localStorage) once, on release — writing it
  // per pointermove was a synchronous storage write every frame.
  const [resizing, setResizing] = useState(false);
  const [dragWidth, setDragWidth] = useState<number | null>(null);
  const dragRef = useRef<{ x: number; width: number; next: number; frame: number } | null>(null);
  const shownWidth = dragWidth ?? width;

  useEffect(() => {
    if (!resizing) return;
    const move = (e: PointerEvent) => {
      const d = dragRef.current;
      if (!d) return;
      // The dock hugs the right edge: dragging its handle left widens it.
      d.next = clampDockWidth(d.width + (d.x - e.clientX));
      // One relayout per frame, however fast the pointer reports.
      if (d.frame) return;
      d.frame = requestAnimationFrame(() => {
        d.frame = 0;
        setDragWidth(d.next);
      });
    };
    const up = () => {
      const d = dragRef.current;
      dragRef.current = null;
      if (d) {
        cancelAnimationFrame(d.frame);
        setWidth(d.next);
      }
      setDragWidth(null);
      setResizing(false);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
    };
  }, [resizing, setWidth]);

  const startDrag = (e: ReactPointerEvent) => {
    e.preventDefault();
    dragRef.current = { x: e.clientX, width, next: width, frame: 0 };
    setResizing(true);
  };

  const close = () => {
    setOpen(false);
    onClose?.();
  };

  // Only tabs that have something in them: a tab whose whole content is
  // "暂无…" is noise.
  const tabs: DockTab[] = [];
  if (changes.length > 0) tabs.push("changes");
  if (artifacts.length > 0) tabs.push("artifacts");
  if (jobs.length > 0) tabs.push("jobs");
  if (subagents.length > 0) tabs.push("subagents");
  const counts: Record<DockTab, number> = {
    changes: changes.length,
    artifacts: artifacts.length,
    jobs: jobs.length,
    subagents: subagents.length,
  };
  // The stored tab can point at the list that just emptied (a new session,
  // /clear) — fall back to whichever one is on screen.
  const activeTab: DockTab | null = tabs.includes(tab) ? tab : (tabs[0] ?? null);

  return (
    <div
      className={cn(
        "flex-none overflow-hidden",
        // The column eases open rather than snapping: WebKit has no scroll
        // anchoring, so a one-frame 300px re-flow of the stream threw the
        // reader's place by however much the text re-wrapped above them —
        // and the dock auto-opens mid-turn, on the first change.
        resizing || snap ? "" : "transition-[width] duration-300 ease-out",
      )}
      style={{ width: open ? shownWidth : 0 }}
      aria-hidden={!open}
    >
      <aside
        style={{ width: shownWidth }}
        className={cn(
          "relative flex h-full flex-none flex-col overflow-hidden border-l border-hairline bg-[color-mix(in_srgb,var(--husk-n50)_70%,transparent)] select-none",
          resizing || snap ? "" : "transition-transform duration-300 ease-out",
          open ? "translate-x-0" : "translate-x-full",
        )}
      >
        {/* Resize handle — a thin lane on the outer edge, so the hit area
            costs no room from the rows. */}
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label="调整侧栏宽度"
          aria-valuenow={shownWidth}
          aria-valuemin={DOCK_MIN_WIDTH}
          tabIndex={0}
          onPointerDown={startDrag}
          onKeyDown={(e) => {
            if (e.key === "ArrowLeft") setWidth(width + 24);
            else if (e.key === "ArrowRight") setWidth(width - 24);
          }}
          className={cn(
            "absolute inset-y-0 left-0 z-10 w-1.5 cursor-col-resize",
            "hover:bg-[color-mix(in_srgb,var(--husk-n300)_60%,transparent)]",
            resizing && "bg-[color-mix(in_srgb,var(--husk-n300)_80%,transparent)]",
          )}
        />

        {/* No rule under the header: the tab track already reads as the bar,
            the same way the session sidebar's header carries its switch. */}
        <div className="flex h-11 w-full flex-none items-center gap-2 pl-3 pr-2">
          <div
            role="tablist"
            aria-label="侧栏"
            className="no-scrollbar flex min-w-0 items-center gap-0.5 overflow-x-auto rounded-full bg-[color-mix(in_srgb,var(--husk-black)_5%,transparent)] p-0.5 dark:bg-[var(--husk-border)]"
          >
            {tabs.map((t) => (
              <TabButton
                key={t}
                tab={t}
                active={activeTab === t}
                files={counts[t]}
                onSelect={setTab}
              />
            ))}
          </div>
          <span className="flex-1" />
          <button
            type="button"
            onClick={close}
            aria-label="关闭侧栏"
            className="flex h-6 w-6 flex-none items-center justify-center rounded-full text-neutral-500 transition-colors hover:bg-[color-mix(in_srgb,var(--husk-black)_5%,transparent)] hover:text-neutral-800"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>

        {/* `overflow-x-hidden` is load-bearing, not cosmetic: a box with
            `overflow-y: auto` and `overflow-x: visible` computes the X axis to
            `auto` too, which turns the whole dock into a horizontal scroll
            container. The horizontal scroller is the diff card itself. */}
        <div className="w-full flex-1 min-h-0 overflow-y-auto overflow-x-hidden px-2 pb-3">
          {activeTab === null ? (
            <div className="px-3 py-10 text-center text-[12px] text-neutral-400">
              本会话还没有文件产出
            </div>
          ) : activeTab === "changes" ? (
            <ChangesList changes={changes} />
          ) : activeTab === "artifacts" ? (
            <ArtifactsList artifacts={artifacts} />
          ) : activeTab === "jobs" ? (
            <JobsList jobs={jobs} snapshotAt={jobsAt} root={root} session={session} onJobs={(j) => onJobs?.(j)} />
          ) : (
            <SubagentsList children={subagents} root={root} session={session} />
          )}
        </div>
      </aside>
    </div>
  );
}
