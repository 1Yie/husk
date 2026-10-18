// Resize edges for the frameless main window.
//
// The runtime's own undecorated resizing only claims a 5px inset and sets the
// pointer from GTK on motion, without ever handing it back — so the edge was
// hard to hit and the resize arrow lingered or flickered against the page's
// own cursor. These strips own the edge instead: the pointer comes from CSS
// (it leaves with the strip), and a press hands the drag to the window
// manager via `startResizeDragging`.
import { useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isMac } from "@/lib/platform";

const win = getCurrentWindow();

type ResizeDirection = Parameters<typeof win.startResizeDragging>[0];

/** Edge strip thickness and corner square size, in CSS px. */
const EDGE = 6;
const CORNER = 12;

const ZONES: { dir: ResizeDirection; cursor: string; style: React.CSSProperties }[] = [
  { dir: "North", cursor: "ns-resize", style: { top: 0, left: CORNER, right: CORNER, height: EDGE } },
  { dir: "South", cursor: "ns-resize", style: { bottom: 0, left: CORNER, right: CORNER, height: EDGE } },
  { dir: "West", cursor: "ew-resize", style: { left: 0, top: CORNER, bottom: CORNER, width: EDGE } },
  { dir: "East", cursor: "ew-resize", style: { right: 0, top: CORNER, bottom: CORNER, width: EDGE } },
  { dir: "NorthWest", cursor: "nwse-resize", style: { top: 0, left: 0, width: CORNER, height: CORNER } },
  { dir: "SouthEast", cursor: "nwse-resize", style: { bottom: 0, right: 0, width: CORNER, height: CORNER } },
  { dir: "NorthEast", cursor: "nesw-resize", style: { top: 0, right: 0, width: CORNER, height: CORNER } },
  { dir: "SouthWest", cursor: "nesw-resize", style: { bottom: 0, left: 0, width: CORNER, height: CORNER } },
];

/** Whether (x, y) lands on a visible scrollbar of some element under the
 *  edges. The strips sit exactly where scrollbars live (the right and bottom
 *  edge), so without this a scrollbar there could only ever resize the window. */
function overScrollbar(x: number, y: number, edges: Set<Element>): boolean {
  if (typeof document.elementsFromPoint !== "function") return false;
  for (const el of document.elementsFromPoint(x, y)) {
    if (edges.has(el) || !(el instanceof HTMLElement)) continue;
    const r = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    const bl = parseFloat(cs.borderLeftWidth) || 0;
    const bt = parseFloat(cs.borderTopWidth) || 0;
    // Gutter = what the box spends beyond its client area on scrollbars.
    const vBar = el.offsetWidth - el.clientWidth - bl - (parseFloat(cs.borderRightWidth) || 0);
    const hBar = el.offsetHeight - el.clientHeight - bt - (parseFloat(cs.borderBottomWidth) || 0);
    if (vBar > 0 && x >= r.left + bl + el.clientWidth && y < r.top + bt + el.clientHeight) return true;
    if (hBar > 0 && y >= r.top + bt + el.clientHeight && x < r.left + bl + el.clientWidth) return true;
  }
  return false;
}

/** Settle delay after the last resize event before the window counts as idle. */
const RESIZE_IDLE_MS = 150;

export function WindowResizeEdges() {
  const [maximized, setMaximized] = useState(false);
  const edgesRef = useRef<(HTMLDivElement | null)[]>([]);

  // Step the strips aside while the pointer is over a scrollbar beneath them:
  // `pointer-events: none` lets the press reach the scrollbar, and the next
  // move off it brings the strips back. Only checked near the window edge.
  useEffect(() => {
    if (isMac || maximized) return;
    const edgeList = () => edgesRef.current.filter((el): el is HTMLDivElement => el !== null);
    let passing = false;
    const onMove = (e: PointerEvent) => {
      if (e.buttons !== 0) return; // a drag in progress keeps its target
      const { clientX: x, clientY: y } = e;
      const nearEdge =
        x < CORNER || y < CORNER || x > window.innerWidth - CORNER || y > window.innerHeight - CORNER;
      const pass = nearEdge && overScrollbar(x, y, new Set(edgeList()));
      if (pass === passing) return;
      passing = pass;
      for (const el of edgeList()) el.style.pointerEvents = pass ? "none" : "";
    };
    document.addEventListener("pointermove", onMove, true);
    return () => {
      document.removeEventListener("pointermove", onMove, true);
      for (const el of edgeList()) el.style.pointerEvents = "";
    };
  }, [maximized]);

  useEffect(() => {
    if (isMac) return;
    let alive = true;
    let idle = 0;
    const root = document.documentElement;
    const sync = () =>
      void win
        .isMaximized()
        .then((m) => alive && setMaximized(m))
        .catch(() => {});
    sync();
    // `data-window-resizing` lets CSS drop transitions and backdrop blurs for
    // the duration of a drag — every frame re-lays out the whole shell.
    const onResize = () => {
      root.dataset.windowResizing = "";
      window.clearTimeout(idle);
      idle = window.setTimeout(() => {
        delete root.dataset.windowResizing;
        sync();
      }, RESIZE_IDLE_MS);
    };
    window.addEventListener("resize", onResize);
    return () => {
      alive = false;
      window.clearTimeout(idle);
      window.removeEventListener("resize", onResize);
      delete root.dataset.windowResizing;
    };
  }, []);

  // macOS keeps native decorations; a maximized window has no edge to drag.
  if (isMac || maximized) return null;

  return (
    <>
      {ZONES.map(({ dir, cursor, style }, i) => (
        <div
          key={dir}
          ref={(el) => {
            edgesRef.current[i] = el;
          }}
          aria-hidden
          data-tauri-drag-region="false"
          className="fixed z-[2147483647]"
          style={{ ...style, cursor }}
          onPointerDown={(e) => {
            if (e.button !== 0) return;
            e.preventDefault();
            void win.startResizeDragging(dir).catch((err) => console.error("resize:", err));
          }}
        />
      ))}
    </>
  );
}
