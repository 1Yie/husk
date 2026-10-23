import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import { TooltipSimple } from "@/components/ui/tooltip";

/** Row height; the strip slides in whole rows. */
const ROW_H = 11;

/** Fewest rows the strip ever claims — also the value before the first
 *  measurement, low enough that it can never overshoot the lane. */
const MIN_ROWS = 9;

/** Rows that fit a lane `h` px tall: the track's 16px of padding, ROW_H a row. */
function rowsForHeight(h: number): number {
  return Math.max(MIN_ROWS, Math.floor((h - 16) / ROW_H));
}

export interface RailMark {
  id: string;
  /** `top` / `end` = the pinned handles; `placeholder` = an unloaded turn (a dot). */
  type: "top" | "end" | "user" | "assistant" | "placeholder";
  /** Ordinal within the unloaded range — click-seek uses it to estimate
   * which history index this placeholder stands for. */
  phIndex?: number;
  /** Owning turn — lets the rail measure the always-mounted shell when
   *  the marked content itself is windowed out. */
  turnId?: string;
  targetId: string;
  previewTitle: string;
  previewSnippet: string;
  /** Weight of the block this mark stands for, in characters (tool cards included). */
  len?: number;
  isStreaming?: boolean;
}

interface ChatTurnRailProps {
  marks: RailMark[];
  activeId: string;
  onSelectMark: (mark: RailMark) => void;
  className?: string;
  /** Session load in flight — the track renders a skeleton strip
   * (pulsing dashes) instead of empty/missing marks. */
  loading?: boolean;
  /** Height of the composer band under the stream. The lane ends there, so the
   *  strip can never be laid over the composer. */
  bottomInset?: number;
}

/**
 * The turn rail: a session minimap down the left edge.
 *
 * Draws a window of rows around the active mark, with a `~` (its count) on
 * either side; wheel and drag slide that window, a click jumps the conversation,
 * and the window follows the active mark.
 */
export function ChatTurnRail({
  marks,
  activeId,
  onSelectMark,
  className,
  loading,
  bottomInset,
}: ChatTurnRailProps) {
  /** Root node as state: the measuring effect must re-attach whenever the node
   *  (re)mounts — a ref leaves the effect with nothing to depend on. */
  const [rootEl, setRootEl] = useState<HTMLDivElement | null>(null);
  /** The drag surface, in state for the same reason as the root. */
  const [trackEl, setTrackEl] = useState<HTMLDivElement | null>(null);
  const activeElRef = useRef<HTMLDivElement>(null);
  const [isDragging, setIsDragging] = useState(false);
  const [rowBudget, setRowBudget] = useState(MIN_ROWS);
  /** Rows the wheel/drag slid the window by; reset when the conversation scrolls. */
  const [browseShift, setBrowseShift] = useState(0);
  const hasDraggedRef = useRef(false);
  const startYRef = useRef(0);
    /** Shift at drag start. */
  const startShiftRef = useRef(0);
    /** Set once the drag is real; the capture waits for it. */
  const capturedRef = useRef(false);
  /** Wheel pixels not yet converted into whole rows. */
  const wheelAccRef = useRef(0);
    /** Latest applied shift (a drag moves relative to it). */
  const browseShiftRef = useRef(0);

  // The budget comes from the lane the rail actually occupies, never from the
  // window: a frameless macOS window can be shown at a height `innerHeight`
  // never reports, and no `resize` follows — the strip then outgrew its
  // container and swallowed clicks meant for the stream.
  useEffect(() => {
    const root = rootEl;
    if (!root) return;
    const observer = new ResizeObserver((entries) => {
      const entry = entries[entries.length - 1];
      const next = rowsForHeight(entry ? entry.contentRect.height : root.clientHeight);
      // Unchanged counts bail out of the render.
      setRowBudget((prev) => (prev === next ? prev : next));
    });
    observer.observe(root);
    return () => observer.disconnect();
  }, [rootEl]);

  useEffect(() => {
    browseShiftRef.current = 0;
    setBrowseShift(0);
  }, [activeId]);

  // Handles pinned, a window between them: the budget leaves room for both handles and a `~` row per side.
  const innerCount = Math.max(0, marks.length - 2);
  const innerSize = Math.max(3, rowBudget - 4);
  const collapsed = innerCount > innerSize;

  /** Slide the window by whole rows — the one gesture behind drag and wheel. */
  const slideRows = (rows: number) => {
    browseShiftRef.current += rows;
    setBrowseShift(browseShiftRef.current);
  };

  // Native and non-passive on the drag SURFACE (capture): React's `onWheel` is
  // passive, and capture runs before any handler that could swallow the event.
  // The surface, not the root: the root is `pointer-events-none` and such an
  // element never receives wheel events. Gated on `collapsed` — with nothing
  // folded there is nothing to slide.
  useEffect(() => {
    const el = trackEl;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (e.ctrlKey || e.deltaY === 0 || !collapsed) return;
      e.preventDefault();
      e.stopPropagation();
      // WebKitGTK reports a notch in lines (deltaMode 1, deltaY ≈ 3), Chromium in pixels — one notch has to move the strip in both.
      const perUnit = e.deltaMode === 1 ? 32 : e.deltaMode === 2 ? el.clientHeight : 1;
      wheelAccRef.current += e.deltaY * perUnit;
      const rows = Math.trunc(wheelAccRef.current / ROW_H);
      if (rows === 0) return;
      wheelAccRef.current -= rows * ROW_H;
      slideRows(rows);
    };
    el.addEventListener("wheel", onWheel, { passive: false, capture: true });
    return () => el.removeEventListener("wheel", onWheel, { capture: true } as any);
  }, [trackEl, collapsed]);

  const handlePointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    hasDraggedRef.current = false;
    capturedRef.current = false;
    startYRef.current = e.clientY;
    startShiftRef.current = browseShift;
    setIsDragging(true);
    // No capture yet: capturing on pointerdown sends the CLICK to the track
    // instead of the mark's row.
  };

  const handlePointerMove = (e: React.PointerEvent) => {
    if (!isDragging) return;
    const dy = e.clientY - startYRef.current;
    if (!collapsed) return;
    if (Math.abs(dy) > 3 && !hasDraggedRef.current) {
      hasDraggedRef.current = true;
      try {
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
        capturedRef.current = true;
      } catch {}
    }
    if (!hasDraggedRef.current) return;
    // Grab-and-pull walks back through the session; the wheel is the same gesture in steps.
    const wanted = startShiftRef.current - Math.round(dy / ROW_H);
    slideRows(wanted - browseShiftRef.current);
    browseShiftRef.current = wanted;
  };

  const handlePointerUp = (e: React.PointerEvent) => {
    if (!isDragging) return;
    setIsDragging(false);
    if (capturedRef.current) {
      capturedRef.current = false;
      try {
        (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
      } catch {}
    }
  };

  if (loading) {
    // Skeleton strip — same lane and pill shape as the real track.
    return (
      <div
        style={{ bottom: bottomInset }}
        className={cn(
          "absolute left-3 inset-y-0 z-[15] flex flex-col items-center justify-center select-none pointer-events-none",
          className
        )}
      >
        <div className="flex flex-col items-center py-2 px-1.5 rounded-full gap-0 max-h-full overflow-hidden">
          {[0, 1, 2, 3, 4, 5, 6].map((i) => (
            <div key={i} className="h-[11px] w-[20px] flex items-center justify-center">
              <div
                className="w-[10px] h-[2px] rounded-full bg-neutral-300 dark:bg-[var(--husk-n300)] animate-pulse"
                style={{ animationDelay: `${i * 90}ms` }}
              />
            </div>
          ))}
        </div>
      </div>
    );
  }

  if (marks.length <= 1) {
    return null;
  }

  const activeIdx = Math.max(
    0,
    marks.findIndex((m) => m.id === activeId)
  );
  const inner = collapsed ? marks.slice(1, marks.length - 1) : [];
  const activeInner = Math.min(Math.max(activeIdx - 1, 0), Math.max(0, innerCount - 1));
  // Centre first, then shift: shifting before the clamp is swallowed near the
  // ends, where the centred window is already pinned.
  const lastStart = Math.max(0, innerCount - innerSize);
  const centredStart = Math.min(Math.max(activeInner - Math.floor(innerSize / 2), 0), lastStart);
  const winStart = collapsed ? Math.min(Math.max(centredStart + browseShift, 0), lastStart) : 0;
  const shown = collapsed ? inner.slice(winStart, winStart + innerSize) : marks.slice(1, marks.length - 1);
  const hiddenAbove = collapsed ? winStart : 0;
  const hiddenBelow = collapsed ? Math.max(0, innerCount - (winStart + innerSize)) : 0;

  const renderMark = (mark: RailMark) => {
    const isActive = mark.id === activeId;

    return (
      <TooltipSimple
        key={mark.id}
        side="right"
        sideOffset={10}
        className="max-w-[280px]"
        content={
          <div className="flex max-w-[260px] flex-col gap-0.5">
            <div className="text-[11px] font-medium">{mark.previewTitle}</div>
            {mark.previewSnippet && (
              <div className="truncate text-[12px] font-normal text-zinc-300">
                {mark.previewSnippet}
              </div>
            )}
          </div>
        }
      >
        <div
          ref={isActive ? activeElRef : undefined}
          onClick={(e) => {
            e.stopPropagation();
            if (!hasDraggedRef.current) {
              onSelectMark(mark);
            }
          }}
          className="h-[11px] w-[20px] flex items-center justify-center group/item relative cursor-pointer pointer-events-auto"
        >
        {mark.type === "top" || mark.type === "end" ? (
          // The two handles — 4 dashed dots at either end of the strip
          <div
            className={cn(
              "flex items-center justify-center gap-[1.5px] transition-all duration-150",
              isActive ? "opacity-100 scale-110" : "opacity-40 group-hover/item:opacity-90"
            )}
          >
            {[0, 1, 2, 3].map((i) => (
              <span
                key={i}
                className={cn(
                  "rounded-full transition-colors",
                  isActive
                    ? "w-[2.5px] h-[3px] bg-neutral-900 shadow-[0_0_3px_rgba(255,255,255,0.35)]"
                    : "w-[2px] h-[2px] bg-neutral-600 group-hover/item:bg-neutral-800"
                )}
              />
            ))}
          </div>
        ) : mark.type === "placeholder" ? (
          // Unloaded turn — a small dot ("点点点" column fills the rail's
          // unloaded share; click pages up to that region).
          <div
            className={cn(
              "rounded-full transition-all duration-150",
              "w-[3px] h-[3px] bg-neutral-300 dark:bg-[var(--husk-n300)] group-hover/item:bg-neutral-500 dark:group-hover/item:bg-[var(--husk-n400)] group-hover/item:w-[4px] group-hover/item:h-[4px]"
            )}
          />
        ) : mark.type === "user" ? (
          // User mark: long dash (14×2px; active 14×3.5px)
          <div
            className={cn(
              "rounded-full transition-all duration-150",
              isActive
                ? "w-[14px] h-[3.5px] bg-neutral-900 shadow-[0_0_4px_rgba(255,255,255,0.35)]"
                : "w-[14px] h-[2px] bg-neutral-300 dark:bg-[var(--husk-n300)] group-hover/item:bg-neutral-500 dark:group-hover/item:bg-[var(--husk-n400)] group-hover/item:w-[16px]"
            )}
          />
        ) : (
          // Assistant mark: short dash (8×2px; active 9×3.5px)
          <div
            className={cn(
              "rounded-full transition-all duration-150",
              isActive
                ? "w-[9px] h-[3.5px] bg-neutral-900 shadow-[0_0_4px_rgba(255,255,255,0.35)]"
                : "w-[8px] h-[2px] bg-neutral-300 dark:bg-[var(--husk-n300)] group-hover/item:bg-neutral-500 dark:group-hover/item:bg-[var(--husk-n400)] group-hover/item:w-[10px]"
            )}
          />
        )}

        </div>
      </TooltipSimple>
    );
  };

  const renderGap = (count: number, side: "above" | "below") => {
    const label = side === "above" ? "上方" : "下方";
    return (
      <TooltipSimple key={`gap-${side}`} side="right" sideOffset={10} content={`${label}还有 ${count} 个标记`}>
      <div
        onClick={(e) => {
          e.stopPropagation();
          if (!hasDraggedRef.current) {
            // Jumping lands on the mark nearest the fold, which pulls that
            // region into the window — the folded part opens.
            const mark = side === "above" ? inner[winStart - 1] : inner[winStart + innerSize];
            if (mark) onSelectMark(mark);
          }
        }}
        className="h-[11px] w-[20px] flex items-center justify-center group/item relative cursor-pointer pointer-events-auto"
        aria-label={`${label}还有 ${count} 个标记`}
      >
        <span className="select-none text-[11px] leading-none font-medium text-neutral-400 transition-colors group-hover/item:text-neutral-700 dark:text-[var(--husk-n400)] dark:group-hover/item:text-[var(--husk-n200)]">
          ~
        </span>
      </div>
      </TooltipSimple>
    );
  };

  const rows: React.ReactNode[] = [];
  rows.push(renderMark(marks[0]));
  if (hiddenAbove > 0) rows.push(renderGap(hiddenAbove, "above"));
  shown.forEach((mark) => rows.push(renderMark(mark)));
  if (hiddenBelow > 0) rows.push(renderGap(hiddenBelow, "below"));
  rows.push(renderMark(marks[marks.length - 1]));

  return (
    <div
      ref={setRootEl}
      style={{ bottom: bottomInset }}
      className={cn(
        // The lane: the stream's height above the composer, never the window's.
        // `pointer-events-none` so the strip can only take clicks on its rows.
        "absolute left-3 inset-y-0 z-[15] flex flex-col items-center select-none pointer-events-none",
        className
      )}
    >
      <div className="h-full flex flex-col justify-center">
        <div
          className={cn(
            // `overflow-hidden` is a hard bound so a wrong budget can never draw
            // the strip outside this lane; the hover cards are portalled, so
            // clipping them is not a concern here.
            "relative flex flex-col items-center py-2 px-1.5 rounded-full transition-colors pointer-events-none max-h-full overflow-hidden",
            isDragging
              ? "bg-[color-mix(in_srgb,var(--husk-n200)_50%,transparent)]"
              : "hover:bg-[color-mix(in_srgb,var(--husk-n100)_50%,transparent)]"
          )}
        >
          {/* The drag surface: exactly the rows, and the only part of the rail
              that takes the pointer (and so the only part that gets wheel). */}
          <div
            ref={setTrackEl}
            onPointerDown={handlePointerDown}
            onPointerMove={handlePointerMove}
            onPointerUp={handlePointerUp}
            onPointerCancel={handlePointerUp}
            className="flex flex-col items-center touch-none cursor-pointer pointer-events-auto"
          >
            {rows}
          </div>
        </div>
      </div>
    </div>
  );
}
