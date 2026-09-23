import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";

/** One mark's row height — the unit the strip scrolls in. */
const ROW_H = 11;

/** Rows the track can draw before it has to fold: the shell's own
 *  `calc(100vh - 220px)` cap, minus its 16px of vertical padding, at 11px a
 *  row. Recomputed on resize — a shorter window shows a shorter strip. */
function rowsThatFit(): number {
  const vh = typeof window === "undefined" ? 900 : window.innerHeight;
  return Math.max(9, Math.floor((vh - 220 - 16) / ROW_H));
}

export interface RailMark {
  id: string;
  /** `top` / `end` are the two handles (start and end of the session);
   *  `placeholder` is an unloaded turn, drawn as a dot. */
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
  /** Weight of the block this mark stands for (characters, tool cards
   *  included) — carried for a future proportional dash; the renderer sizes
   *  marks by role alone today. */
  len?: number;
  isStreaming?: boolean;
}

/** Hover card shared by the marks and the `~` folds: one label line plus the
 *  block's own text. */
function RailTip({ title, detail }: { title: string; detail?: string }) {
  return (
    <div className="pointer-events-none absolute left-7 top-1/2 -translate-y-1/2 z-50 flex flex-col gap-0.5 whitespace-nowrap bg-[color-mix(in_srgb,var(--husk-n900)_95%,transparent)] dark:bg-[var(--husk-card)]/95 text-zinc-50 px-2.5 py-1.5 rounded-lg shadow-popup border border-zinc-700/60 backdrop-blur-xs max-w-[280px] animate-in fade-in-0 zoom-in-95 duration-100">
      <div className="text-[11px] font-medium text-zinc-100">{title}</div>
      {detail && (
        <div className="truncate text-[12px] font-normal text-neutral-400 max-w-[260px]">{detail}</div>
      )}
    </div>
  );
}

interface ChatTurnRailProps {
  marks: RailMark[];
  activeId: string;
  onSelectMark: (mark: RailMark) => void;
  className?: string;
  /** Session load in flight — the track renders a skeleton strip
   * (pulsing dashes) instead of empty/missing marks. */
  loading?: boolean;
}

/**
 * The turn rail: a minimap of the session down the left edge.
 *
 * The strip draws a WINDOW of turns — as many rows as the window has room for,
 * with the rest folded behind a `~` on either side — and its own wheel slides
 * that window, so the rail is browsed where the pointer is instead of by
 * scrolling the conversation. The two handles (start / end) stay pinned; a
 * click jumps the conversation to the mark under it; the window follows the
 * active mark whenever the conversation itself moves.
 */
export function ChatTurnRail({
  marks,
  activeId,
  onSelectMark,
  className,
  loading,
}: ChatTurnRailProps) {
  /** The rail's root node, as STATE: the wheel listener has to re-attach when
   *  the node appears or is replaced (skeleton → strip, session switch), and a
   *  plain ref gives the effect nothing to watch — it ran once while the
   *  skeleton was mounted, found no node, and never ran again because
   *  `collapsed` had not changed. */
  const [rootEl, setRootEl] = useState<HTMLDivElement | null>(null);
  const trackRef = useRef<HTMLDivElement>(null);
  const activeElRef = useRef<HTMLDivElement>(null);
  const [isDragging, setIsDragging] = useState(false);
  const [hoveredMarkId, setHoveredMarkId] = useState<string | null>(null);
  const [rowBudget, setRowBudget] = useState(rowsThatFit);
  /** Rows the wheel has slid the window by — browsing the folded regions.
   *  Reset whenever the conversation's own scroll moves the active mark, so
   *  the window snaps back to following the reader. */
  const [browseShift, setBrowseShift] = useState(0);
  const hasDraggedRef = useRef(false);
  const startYRef = useRef(0);
  /** Browse position the current drag started from. */
  const startShiftRef = useRef(0);
  /** Pointer capture is taken only once a drag is under way (see below). */
  const capturedRef = useRef(false);
  /** Wheel pixels not yet converted into whole rows. */
  const wheelAccRef = useRef(0);
  /** Latest applied shift, so a drag can move relative to it. */
  const browseShiftRef = useRef(0);

  useEffect(() => {
    const onResize = () => setRowBudget(rowsThatFit());
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  useEffect(() => {
    browseShiftRef.current = 0;
    setBrowseShift(0);
  }, [activeId]);

  // Window shape: the two handles stay pinned, everything between them is a
  // window. The budget keeps room for both handles and a `~` row per side, so
  // the tallest arrangement still fits the shell's cap.
  const innerCount = Math.max(0, marks.length - 2);
  const innerSize = Math.max(3, rowBudget - 4);
  const collapsed = innerCount > innerSize;

  /** Slide the strip's own window by whole rows. One mechanism behind both the
   *  press-and-drag and the wheel: the strip moves under the pointer, the
   *  conversation stays where it is. */
  const slideRows = (rows: number) => {
    browseShiftRef.current += rows;
    setBrowseShift(browseShiftRef.current);
  };

  // Wheel over the strip slides the strip's own window. A native non-passive
  // listener, not `onWheel`: React registers wheel passively, so the event
  // would reach whatever scroll container sits behind the rail. Gated on
  // `collapsed` — with every mark already drawn there is nothing to slide, and
  // swallowing the event would only hide that.
  useEffect(() => {
    // On the rail's ROOT in the capture phase: the strip's own rows, the `~`
    // folds and the hover cards all sit under it, and a listener further in
    // only sees the events that survive whatever else is listening. Capture
    // also runs before any bubble-phase handler can swallow it.
    const root = rootEl;
    if (!root) return;
    const onWheel = (e: WheelEvent) => {
      if (e.ctrlKey || e.deltaY === 0 || !collapsed) return;
      e.preventDefault();
      e.stopPropagation();
      // WebKitGTK reports a mouse notch in LINES (deltaMode 1, deltaY ≈ 3),
      // Chromium in pixels. Converting each to pixels is what makes one notch
      // move the strip in either engine.
      const perUnit = e.deltaMode === 1 ? 32 : e.deltaMode === 2 ? root.clientHeight : 1;
      wheelAccRef.current += e.deltaY * perUnit;
      const rows = Math.trunc(wheelAccRef.current / ROW_H);
      if (rows === 0) return;
      wheelAccRef.current -= rows * ROW_H;
      slideRows(rows);
    };
    root.addEventListener("wheel", onWheel, { passive: false, capture: true });
    return () => root.removeEventListener("wheel", onWheel, { capture: true } as any);
  }, [rootEl, collapsed]);

  const handlePointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    hasDraggedRef.current = false;
    capturedRef.current = false;
    startYRef.current = e.clientY;
    startShiftRef.current = browseShift;
    setIsDragging(true);
    // No capture yet: capturing on pointerdown makes the browser dispatch the
    // CLICK to the capture target (the track) instead of the row, so a plain
    // click never reached the mark it was on.
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
    // Grabbing the strip and pulling down walks back through the session, the
    // way dragging a list does — the wheel is the same gesture in discrete
    // steps.
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
    // Skeleton strip — same pill shape as the real track.
    return (
      <div
        className={cn(
          "absolute left-3 top-1/2 -translate-y-1/2 z-30 flex flex-col items-center select-none",
          className
        )}
      >
        <div className="flex flex-col items-center py-2 px-1.5 rounded-full gap-0">
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
  // Centre on the active mark FIRST, then apply the wheel's shift. Shifting
  // before the clamp swallowed the scroll whenever the active mark sat near an
  // end: the centred window was already pinned there, so a nudge landed on the
  // same clamped value and the strip looked frozen.
  const lastStart = Math.max(0, innerCount - innerSize);
  const centredStart = Math.min(Math.max(activeInner - Math.floor(innerSize / 2), 0), lastStart);
  const winStart = collapsed ? Math.min(Math.max(centredStart + browseShift, 0), lastStart) : 0;
  const shown = collapsed ? inner.slice(winStart, winStart + innerSize) : marks.slice(1, marks.length - 1);
  const hiddenAbove = collapsed ? winStart : 0;
  const hiddenBelow = collapsed ? Math.max(0, innerCount - (winStart + innerSize)) : 0;

  const renderMark = (mark: RailMark) => {
    const isActive = mark.id === activeId;
    const isHovered = mark.id === hoveredMarkId && !isDragging;

    return (
      <div
        key={mark.id}
        ref={isActive ? activeElRef : undefined}
        onMouseEnter={() => setHoveredMarkId(mark.id)}
        onMouseLeave={() => setHoveredMarkId(null)}
        onClick={(e) => {
          e.stopPropagation();
          if (!hasDraggedRef.current) {
            onSelectMark(mark);
          }
        }}
        className="h-[11px] w-[20px] flex items-center justify-center group/item relative cursor-pointer"
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

        {isHovered && <RailTip title={mark.previewTitle} detail={mark.previewSnippet} />}
      </div>
    );
  };

  const renderGap = (count: number, side: "above" | "below") => {
    const label = side === "above" ? "上方" : "下方";
    return (
      <div
        key={`gap-${side}`}
        onMouseEnter={() => setHoveredMarkId(`gap-${side}`)}
        onMouseLeave={() => setHoveredMarkId(null)}
        onClick={(e) => {
          e.stopPropagation();
          if (!hasDraggedRef.current) {
            // Jumping lands on the mark nearest the fold, which pulls that
            // region into the window — the folded part opens.
            const mark = side === "above" ? inner[winStart - 1] : inner[winStart + innerSize];
            if (mark) onSelectMark(mark);
          }
        }}
        className="h-[11px] w-[20px] flex items-center justify-center group/item relative cursor-pointer"
        aria-label={`${label}还有 ${count} 个标记`}
      >
        <span className="select-none text-[11px] leading-none font-medium text-neutral-400 transition-colors group-hover/item:text-neutral-700 dark:text-[var(--husk-n400)] dark:group-hover/item:text-[var(--husk-n200)]">
          ~
        </span>
        {hoveredMarkId === `gap-${side}` && !isDragging && (
          <RailTip title={`${label}还有 ${count} 个标记`} />
        )}
      </div>
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
      className={cn(
        "absolute left-3 top-1/2 -translate-y-1/2 z-30 flex flex-col items-center select-none",
        className
      )}
    >
      <div
        ref={trackRef}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerCancel={handlePointerUp}
        className={cn(
          // No `overflow` here: the strip is windowed to fit, and the hover
          // cards are `absolute` children — `overflow-y: auto` clipped every
          // one of them to an empty sliver.
          "relative flex flex-col items-center py-2 px-1.5 rounded-full transition-colors touch-none cursor-pointer",
          isDragging
            ? "bg-[color-mix(in_srgb,var(--husk-n200)_50%,transparent)]"
            : "hover:bg-[color-mix(in_srgb,var(--husk-n100)_50%,transparent)]"
        )}
      >
        <div className="flex flex-col items-center">{rows.map((row) => row)}</div>
      </div>
    </div>
  );
}
