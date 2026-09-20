import { useEffect, useRef, useState, useCallback } from "react";
import { cn } from "@/lib/utils";

export interface RailMark {
  id: string;
  type: "top" | "user" | "assistant";
  targetId: string;
  previewTitle: string;
  previewSnippet: string;
  isStreaming?: boolean;
}

interface ChatTurnRailProps {
  marks: RailMark[];
  activeId: string;
  onSelectMark: (mark: RailMark) => void;
  onDragScroll: (ratio: number) => void;
  className?: string;
}

export function ChatTurnRail({
  marks,
  activeId,
  onSelectMark,
  onDragScroll,
  className,
}: ChatTurnRailProps) {
  const trackRef = useRef<HTMLDivElement>(null);
  const [isDragging, setIsDragging] = useState(false);
  const [hoveredMarkId, setHoveredMarkId] = useState<string | null>(null);
  const hasDraggedRef = useRef(false);
  const startYRef = useRef(0);

  // If the rail overflows on long conversations, keep active mark visible
  const activeElRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (activeElRef.current && !isDragging) {
      activeElRef.current.scrollIntoView({ block: "nearest", behavior: "smooth" });
    }
  }, [activeId, isDragging]);

  const updateScrollFromPointer = useCallback(
    (clientY: number) => {
      const track = trackRef.current;
      if (!track) return;
      const rect = track.getBoundingClientRect();
      if (rect.height <= 0) return;
      const relY = clientY - rect.top;
      const ratio = Math.max(0, Math.min(1, relY / rect.height));
      onDragScroll(ratio);
    },
    [onDragScroll]
  );

  const handlePointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    hasDraggedRef.current = false;
    startYRef.current = e.clientY;
    setIsDragging(true);
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    updateScrollFromPointer(e.clientY);
  };

  const handlePointerMove = (e: React.PointerEvent) => {
    if (!isDragging) return;
    if (Math.abs(e.clientY - startYRef.current) > 3) {
      hasDraggedRef.current = true;
    }
    updateScrollFromPointer(e.clientY);
  };

  const handlePointerUp = (e: React.PointerEvent) => {
    if (!isDragging) return;
    setIsDragging(false);
    try {
      (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {}
  };

  if (marks.length <= 1) {
    return null;
  }

  return (
    <div
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
          "relative flex flex-col items-center py-2 px-1.5 rounded-full transition-colors touch-none cursor-pointer max-h-[calc(100vh-220px)] overflow-y-auto no-scrollbar",
          isDragging
            ? "bg-neutral-200/50 dark:bg-neutral-800/60"
            : "hover:bg-neutral-100/50 dark:hover:bg-neutral-800/40"
        )}
      >
        <div className="flex flex-col items-center">
          {marks.map((mark) => {
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
                {mark.type === "top" ? (
                  // Top mark: 4 dashed dots
                  <div
                    className={cn(
                      "flex items-center justify-center gap-[1.5px] transition-all duration-150",
                      isActive
                        ? "opacity-100 scale-110"
                        : "opacity-40 group-hover/item:opacity-90"
                    )}
                  >
                    {[0, 1, 2, 3].map((i) => (
                      <span
                        key={i}
                        className={cn(
                          "rounded-full transition-colors",
                          isActive
                            ? "w-[2.5px] h-[3px] bg-neutral-900 dark:bg-white shadow-[0_0_3px_rgba(255,255,255,0.35)]"
                            : "w-[2px] h-[2px] bg-neutral-600 dark:bg-neutral-400 group-hover/item:bg-neutral-800 dark:group-hover/item:bg-neutral-200"
                        )}
                      />
                    ))}
                  </div>
                ) : mark.type === "user" ? (
                  // User mark: Long dash (14px wide, 2px high -> active: 14px wide, 3.5px high pill)
                  <div
                    className={cn(
                      "rounded-full transition-all duration-150",
                      isActive
                        ? "w-[14px] h-[3.5px] bg-neutral-900 dark:bg-white shadow-[0_0_4px_rgba(255,255,255,0.35)]"
                        : "w-[14px] h-[2px] bg-neutral-300 dark:bg-[#383838] group-hover/item:bg-neutral-500 dark:group-hover/item:bg-[#606060] group-hover/item:w-[16px]"
                    )}
                  />
                ) : (
                  // Assistant mark: Short dash (8px wide, 2px high -> active: 9px wide, 3.5px high pill)
                  <div
                    className={cn(
                      "rounded-full transition-all duration-150",
                      isActive
                        ? "w-[9px] h-[3.5px] bg-neutral-900 dark:bg-white shadow-[0_0_4px_rgba(255,255,255,0.35)]"
                        : "w-[8px] h-[2px] bg-neutral-300 dark:bg-[#383838] group-hover/item:bg-neutral-500 dark:group-hover/item:bg-[#606060] group-hover/item:w-[10px]"
                    )}
                  />
                )}

                {isHovered && (
                  <div className="pointer-events-none absolute left-7 top-1/2 -translate-y-1/2 z-50 flex flex-col gap-0.5 whitespace-nowrap bg-neutral-900/95 dark:bg-[#18181b]/95 text-white text-[11px] px-2.5 py-1.5 rounded-lg shadow-popup border border-neutral-700/60 backdrop-blur-xs max-w-[280px] animate-in fade-in-0 zoom-in-95 duration-100">
                    <div className="flex items-center gap-1.5 text-[10px] font-medium text-neutral-400">
                      <span>{mark.previewTitle}</span>
                      <span className="text-neutral-500">•</span>
                      <span className="text-neutral-400">点击跳转</span>
                    </div>
                    {mark.previewSnippet && (
                      <div className="text-neutral-200 truncate font-normal text-[12px] max-w-[260px]">
                        {mark.previewSnippet}
                      </div>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
