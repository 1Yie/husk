// SessionDock — the one right-hand sidebar. Everything a session produces
// hangs off it: the files the agent changed (改动), the files it generated
// (产物), the background jobs it started (后台) and the subagents it delegated
// (子代理), as tabs. It is independent of the workspace mode (编程 and 工作 both
// get the same dock) and owns its own UI state — open, active tab, width —
// persisted across sessions.
//
// The frame is the dock's; the tab bodies are `ChangesList` / `ArtifactsList`
// / `JobsList` / `SubagentsList`.

import {
  lazy,
  Suspense,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { DockTabs } from "./dock-tabs";
import { WorkspaceTools } from "./workspace-tools";
import "./dock.css";
import { DirectoryLoadingSkeleton } from "@/features/workspace-files/loading";
import { cn } from "@/lib/utils";
import { ChangesList } from "@/features/chat/components/changes-panel";
import { ArtifactsList } from "@/features/chat/components/artifacts-panel";
import { JobsList } from "@/features/chat/components/jobs-panel";
import { SubagentsList } from "@/features/chat/components/subagents-panel";
import type {
  Artifact,
  BgJob,
  FileChange,
} from "@/features/chat/hooks/stream-view";
import type { SessionChild } from "@/features/chat/hooks/running-children";
import type { ToolChipRow } from "@/features/chat/components/tool-chips/index";
import {
  DOCK_MIN_WIDTH,
  availableDockTabs,
  clampDockWidth,
  resolveDockTab,
  useDockStore,
  type DockTab,
} from "./state";

const WorkspaceFiles = lazy(() =>
  import("@/features/workspace-files").then((module) => ({
    default: module.WorkspaceFiles,
  })),
);


export function SessionDock({
  changes,
  artifacts,
  jobs = [],
  jobsAt = 0,
  subagents = [],
  childRows,
  root = "",
  session = 0,
  onJobs,
  open: openProp,
  snap = false,
  onClose,
  onOpenWorkspace,
  officeMode = false,
}: {
  changes: FileChange[];
  artifacts: Artifact[];
  /** The session's background jobs, and when that snapshot arrived. */
  jobs?: BgJob[];
  jobsAt?: number;
  /** Every delegated child, finished ones included — their cards, and a
   *  steer box on the ones still out. */
  subagents?: SessionChild[];
  /** Each child's own card by id — the transcript behind its 工作过程. */
  childRows?: Map<number, ToolChipRow>;
  /** Which session the 后台 controls address. */
  root?: string;
  session?: number;
  /** A control call's returned list — written back to the session's view. */
  onJobs?: (jobs: BgJob[]) => void;
  /** Whether the sidebar is shown. Defaults to the store flag. Workspace
   *  tools are available even without session-generated content. */
  open?: boolean;
  /** Apply open/close instantly, without the slide — set by the page for the
   *  commit that follows a session switch, which is a jump, not a toggle. */
  snap?: boolean;
  /** A user close — the page uses it to suppress this session's auto-open. */
  onClose?: () => void;
  onOpenWorkspace?: () => void;
  officeMode?: boolean;
}) {
  const storeOpen = useDockStore((s) => s.open);
  const open = openProp ?? storeOpen;
  const tab = useDockStore((s) => s.tab);
  const width = useDockStore((s) => s.width);
  const setTab = useDockStore((s) => s.setTab);
  const setWidth = useDockStore((s) => s.setWidth);
  const browserOpen = useDockStore((s) => s.browserOpen);
  const closeBrowser = useDockStore((s) => s.closeBrowser);

  // Width is dragged; a CSS transition would fight the pointer, so the slide
  // is switched off for the duration of the drag. The live width stays local
  // and lands in the store (and localStorage) once, on release — writing it
  // per pointermove was a synchronous storage write every frame.
  const [resizing, setResizing] = useState(false);
  const [dragWidth, setDragWidth] = useState<number | null>(null);
  const dragRef = useRef<{
    x: number;
    width: number;
    next: number;
    frame: number;
  } | null>(null);
  const shownWidth = dragWidth ?? width;
  const wasOpen = useRef(open);

  // The persistent title-bar toggle is the close control. Notify the page
  // before its auto-open effect runs, so a manual close stays closed.
  useLayoutEffect(() => {
    if (wasOpen.current && !open) onClose?.();
    wasOpen.current = open;
  }, [open, onClose]);

  useEffect(() => {
    if (!resizing || !open) {
      if (resizing) {
        setDragWidth(null);
        setResizing(false);
      }
      return;
    }
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
    window.addEventListener("lostpointercapture", up, true);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      window.removeEventListener("lostpointercapture", up, true);
      const d = dragRef.current;
      dragRef.current = null;
      if (d) cancelAnimationFrame(d.frame);
    };
  }, [open, resizing, setWidth]);

  const startDrag = (e: ReactPointerEvent) => {
    e.preventDefault();
    dragRef.current = { x: e.clientX, width, next: width, frame: 0 };
    setResizing(true);
  };

  const counts: Record<DockTab, number> = {
    tools: 0,
    files: 0,
    changes: changes.length,
    artifacts: artifacts.length,
    jobs: jobs.length,
    subagents: subagents.length,
  };
  const tabs = availableDockTabs(counts, { browserOpen, officeMode });
  // The stored tab can point at the list that just emptied (a new session,
  // /clear) — fall back to whichever one is on screen.
  const activeTab = resolveDockTab(tab, counts, { browserOpen, officeMode });

  return (
    <div
      className={cn(
        "flex-none overflow-hidden",
        // The column eases open rather than snapping: WebKit has no scroll
        // anchoring, so a one-frame 300px re-flow of the stream threw the
        // reader's place by however much the text re-wrapped above them —
        // and the dock auto-opens mid-turn, on the first change.
        resizing || snap
          ? ""
          : "transition-[width] duration-300 ease-out motion-reduce:transition-none",
      )}
      style={{ width: open ? shownWidth : 0 }}
      aria-hidden={!open}
      {...(!open ? { inert: "" } : {})}
    >
      <aside
        id="workspace-tools-sidebar"
        aria-label="工作区工具侧栏"
        style={{ width: shownWidth }}
        className={cn(
          "relative flex h-full flex-none flex-col overflow-hidden border-l border-hairline bg-[color-mix(in_srgb,var(--husk-n50)_70%,transparent)] select-none",
          resizing || snap
            ? ""
            : "transition-transform duration-300 ease-out motion-reduce:transition-none",
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
          data-resizing={resizing && open ? "" : undefined}
          onPointerDown={startDrag}
          onKeyDown={(e) => {
            if (e.key === "ArrowLeft") setWidth(width + 24);
            else if (e.key === "ArrowRight") setWidth(width - 24);
          }}
          className="workspace-dock__separator absolute inset-y-0 left-0 z-10 w-1.5 cursor-col-resize"
        />

        {tabs.length > 0 && (
          <DockTabs tabs={tabs} counts={counts} active={activeTab} onSelect={setTab} />
        )}

        {/* `overflow-x-hidden` is load-bearing, not cosmetic: a box with
            `overflow-y: auto` and `overflow-x: visible` computes the X axis to
            `auto` too, which turns the whole dock into a horizontal scroll
            container. The horizontal scroller is the diff card itself. */}
        <div
          className={cn(
            "w-full flex-1 min-h-0",
            activeTab === "files"
              ? "overflow-hidden"
              : activeTab === "tools"
                ? "overflow-y-auto overflow-x-hidden"
                : "overflow-y-auto overflow-x-hidden px-2 pt-1.5 pb-3",
          )}
        >
          {activeTab === "tools" ? (
            <WorkspaceTools
              root={root}
              hasChanges={changes.length > 0}
              officeMode={officeMode}
              onReview={() => setTab("changes")}
              onFiles={() => setTab("files")}
              onOpenWorkspace={onOpenWorkspace}
            />
          ) : activeTab === "files" ? (
            <Suspense fallback={<DirectoryLoadingSkeleton header />}>
              <WorkspaceFiles
                key={root}
                root={root}
                onOpenWorkspace={onOpenWorkspace}
                onBack={closeBrowser}
              />
            </Suspense>
          ) : activeTab === null ? (
            <div className="px-3 py-10 text-center text-[12px] text-neutral-400">
              本会话还没有文件产出
            </div>
          ) : activeTab === "changes" ? (
            <ChangesList changes={changes} />
          ) : activeTab === "artifacts" ? (
            <ArtifactsList artifacts={artifacts} />
          ) : activeTab === "jobs" ? (
            <JobsList
              jobs={jobs}
              snapshotAt={jobsAt}
              root={root}
              session={session}
              onJobs={(j) => onJobs?.(j)}
            />
          ) : (
            <SubagentsList
              children={subagents}
              childRows={childRows}
              root={root}
              session={session}
            />
          )}
        </div>
      </aside>
    </div>
  );
}
