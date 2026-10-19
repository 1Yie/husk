// SessionDock — the one right-hand sidebar. Everything a session produces
// hangs off it: the files the agent changed (改动) and the files it generated
// (产物), as tabs. It is independent of the workspace mode (编程 and 工作 both
// get the same dock) and owns its own UI state — open, active tab, width —
// persisted across sessions.
//
// The frame is the dock's; the tab bodies are `ChangesList` / `ArtifactsList`.

import { useCallback, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { FileDiff, Package, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { ChangesList } from "@/features/chat/components/changes-panel";
import { ArtifactsList } from "@/features/chat/components/artifacts-panel";
import type { Artifact, FileChange } from "@/features/chat/hooks/stream-view";
import { DOCK_MIN_WIDTH, clampDockWidth, useDockStore, type DockTab } from "./state";

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
  const Icon = tab === "changes" ? FileDiff : Package;
  const label = tab === "changes" ? "改动" : "产物";
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      aria-label={`${label}（${files} 个文件）`}
      onClick={() => onSelect(tab)}
      className={cn(
        "relative flex flex-none items-center gap-1.5 rounded-md px-2 py-1 text-[12px] transition-colors",
        active
          ? "bg-[color-mix(in_srgb,var(--husk-n200)_75%,transparent)] font-medium text-neutral-800"
          : "text-neutral-500 hover:bg-[color-mix(in_srgb,var(--husk-n200)_40%,transparent)] hover:text-neutral-700",
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
  open: openProp,
  onClose,
}: {
  changes: FileChange[];
  artifacts: Artifact[];
  /** Whether the sidebar is shown. Defaults to the store flag; the page
   *  additionally ANDs in "has content" so an empty dock never occupies the
   *  column (a fresh session inherits the previous one's open flag). */
  open?: boolean;
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

  // Width is dragged; a CSS width transition would fight the pointer, so the
  // transition is switched off for the duration of the drag.
  const [resizing, setResizing] = useState(false);
  const dragRef = useRef<{ x: number; width: number } | null>(null);

  const stopDrag = useCallback(() => {
    dragRef.current = null;
    setResizing(false);
  }, []);

  useEffect(() => {
    if (!resizing) return;
    const move = (e: PointerEvent) => {
      const d = dragRef.current;
      if (!d) return;
      // The dock hugs the right edge: dragging its handle left widens it.
      setWidth(clampDockWidth(d.width + (d.x - e.clientX)));
    };
    const up = () => stopDrag();
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
    };
  }, [resizing, setWidth, stopDrag]);

  const startDrag = (e: ReactPointerEvent) => {
    e.preventDefault();
    dragRef.current = { x: e.clientX, width };
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
  // The stored tab can point at the list that just emptied (a new session,
  // /clear) — fall back to whichever one is on screen.
  const activeTab: DockTab | null = tabs.includes(tab) ? tab : (tabs[0] ?? null);
  const adds = activeTab === "changes" ? changes.reduce((n, c) => n + c.adds, 0) : 0;
  const dels = activeTab === "changes" ? changes.reduce((n, c) => n + c.dels, 0) : 0;

  return (
    <div
      className={cn(
        "flex-none overflow-hidden",
        resizing ? "" : "transition-[width] duration-300 ease-out",
      )}
      style={{ width: open ? width : 0 }}
      aria-hidden={!open}
    >
      <aside
        style={{ width }}
        className={cn(
          "relative flex h-full flex-none flex-col overflow-hidden border-l border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)] bg-[color-mix(in_srgb,var(--husk-n50)_70%,transparent)] select-none",
          resizing ? "" : "transition-transform duration-300 ease-out",
          open ? "translate-x-0" : "translate-x-full",
        )}
      >
        {/* Resize handle — a thin lane on the outer edge, so the hit area
            costs no room from the rows. */}
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label="调整侧栏宽度"
          aria-valuenow={width}
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

        <div className="flex h-9 w-full flex-none items-center gap-1 border-b border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)] pl-3 pr-2">
          <div role="tablist" aria-label="侧栏" className="flex items-center gap-0.5">
            {tabs.map((t) => (
              <TabButton
                key={t}
                tab={t}
                active={activeTab === t}
                files={t === "changes" ? changes.length : artifacts.length}
                onSelect={setTab}
              />
            ))}
          </div>
          <span className="flex-1" />
          {(adds > 0 || dels > 0) && (
            <span className="font-mono text-[10.5px] tabular-nums">
              {adds > 0 && (
                <span className="text-emerald-600 dark:text-emerald-400">+{adds}</span>
              )}
              {dels > 0 && (
                <span className="ml-1 text-rose-600 dark:text-rose-400">-{dels}</span>
              )}
            </span>
          )}
          <button
            type="button"
            onClick={close}
            aria-label="关闭侧栏"
            className="flex-none rounded p-1 text-neutral-500 transition-colors hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] hover:text-neutral-700"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>

        {/* `overflow-x-hidden` is load-bearing, not cosmetic: a box with
            `overflow-y: auto` and `overflow-x: visible` computes the X axis to
            `auto` too, which turns the whole dock into a horizontal scroll
            container. The horizontal scroller is the diff card itself. */}
        <div className="w-full flex-1 min-h-0 overflow-y-auto overflow-x-hidden px-1.5 py-1.5">
          {activeTab === null ? (
            <div className="px-3 py-8 text-center text-[12px] text-neutral-400">
              本会话还没有文件产出
            </div>
          ) : activeTab === "changes" ? (
            <ChangesList changes={changes} />
          ) : (
            <ArtifactsList artifacts={artifacts} />
          )}
        </div>
      </aside>
    </div>
  );
}
