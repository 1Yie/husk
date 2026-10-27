import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";

type Drag = {
  pointer: number; key: number; startX: number; startY: number;
  x: number; y: number; started: boolean; before: number | null;
};
const TAB = "[data-terminal-tab-key]";
const keyOf = (node: HTMLElement) => Number(node.dataset.terminalTabKey);

/** Only tab chrome moves; terminal hosts and their session keys stay untouched. */
export function TerminalTabStrip({
  active, open, order, reveal, onReorder, children,
}: {
  active: number; open: boolean; order: string; reveal?: { key: number };
  onReorder: (key: number, before: number | null) => void; children: ReactNode;
}) {
  const viewport = useRef<HTMLDivElement>(null);
  const drag = useRef<Drag | null>(null);
  const suppressClick = useRef(false);
  const cancelDrag = useRef<() => void>(() => {});
  const reorder = useRef(onReorder);
  const lastReveal = useRef(reveal);
  reorder.current = onReorder;
  const [edges, setEdges] = useState({ left: false, right: false });
  const measure = () => {
    const node = viewport.current;
    if (!node) return;
    const left = node.scrollLeft > 1;
    const right = node.scrollLeft < node.scrollWidth - node.clientWidth - 1;
    setEdges((old) => old.left === left && old.right === right ? old : { left, right });
  };
  const revealTab = (key: number) => {
    const node = viewport.current;
    const tab = node?.querySelector<HTMLElement>(`${TAB}[data-terminal-tab-key="${key}"]`);
    if (!node || !tab || drag.current?.started) return;
    const clip = node.getBoundingClientRect(), box = tab.getBoundingClientRect();
    // Scroll this viewport, never the page or the terminal's scrollback.
    if (box.left < clip.left) node.scrollLeft -= clip.left - box.left;
    else if (box.right > clip.right) node.scrollLeft += box.right - clip.right;
    measure();
  };
  useLayoutEffect(() => {
    const key = reveal && reveal !== lastReveal.current ? reveal.key : active;
    lastReveal.current = reveal;
    if (open) revealTab(key);
    else cancelDrag.current();
  }, [active, open, order, reveal]);
  useLayoutEffect(() => {
    const node = viewport.current;
    if (!node) return;
    // Child sizes can change after a shell title, rename or font change.
    const observer = new ResizeObserver(() => {
      if (open) revealTab(active);
      measure();
    });
    observer.observe(node);
    node.querySelectorAll<HTMLElement>(TAB).forEach((tab) => observer.observe(tab));
    measure();
    return () => observer.disconnect();
  }, [active, open, children]);
  useEffect(() => {
    const node = viewport.current;
    if (!node) return;
    let frame: number | undefined;
    let previousTime = 0;
    const clearMarkers = () => {
      node.querySelectorAll<HTMLElement>(TAB).forEach((tab) => {
        delete tab.dataset.dragging;
        delete tab.dataset.dropBefore;
        delete tab.dataset.dropAfter;
      });
      delete node.dataset.dragging;
    };
    const cancel = () => {
      if (frame !== undefined) cancelAnimationFrame(frame);
      frame = undefined;
      const pointer = drag.current?.pointer;
      drag.current = null;
      clearMarkers();
      if (pointer !== undefined && node.hasPointerCapture(pointer)) node.releasePointerCapture(pointer);
    };
    cancelDrag.current = cancel;
    const updateTarget = () => {
      const d = drag.current;
      if (!d?.started) return;
      clearMarkers();
      const source = node.querySelector<HTMLElement>(`${TAB}[data-terminal-tab-key="${d.key}"]`);
      if (!source) { cancel(); return; }
      source.dataset.dragging = "true";
      node.dataset.dragging = "true";
      const clip = node.getBoundingClientRect();
      if (d.y < clip.top - 24 || d.y > clip.bottom + 24) return;
      const tabs = [...node.querySelectorAll<HTMLElement>(TAB)].filter((tab) => tab !== source);
      const before = tabs.find((tab) => {
        const box = tab.getBoundingClientRect();
        return d.x < box.left + box.width / 2;
      });
      d.before = before ? keyOf(before) : null;
      if (before) before.dataset.dropBefore = "true";
      else if (tabs.length) tabs[tabs.length - 1].dataset.dropAfter = "true";
    };
    const tick = (time: number) => {
      frame = undefined;
      const d = drag.current;
      if (!d?.started) return;
      const box = node.getBoundingClientRect();
      const elapsed = previousTime ? Math.min(32, time - previousTime) : 16;
      previousTime = time;
      if (d.y >= box.top - 24 && d.y <= box.bottom + 24) {
        const edge = Math.min(36, box.width / 4);
        const speed = d.x < box.left + edge ? -Math.min(1, (box.left + edge - d.x) / edge)
          : d.x > box.right - edge ? Math.min(1, (d.x - box.right + edge) / edge) : 0;
        node.scrollLeft += speed * elapsed * 0.65;
        updateTarget();
      }
      frame = requestAnimationFrame(tick);
    };
    const move = (event: PointerEvent) => {
      const d = drag.current;
      if (!d || d.pointer !== event.pointerId) return;
      d.x = event.clientX; d.y = event.clientY;
      if (!d.started && Math.hypot(d.x - d.startX, d.y - d.startY) < 6) return;
      if (!d.started) {
        d.started = true;
        suppressClick.current = true;
        node.setPointerCapture(event.pointerId);
        previousTime = 0;
        frame = requestAnimationFrame(tick);
      }
      event.preventDefault();
      updateTarget();
    };
    const up = (event: PointerEvent) => {
      const d = drag.current;
      if (!d || d.pointer !== event.pointerId) return;
      d.x = event.clientX; d.y = event.clientY;
      updateTarget();
      const box = node.getBoundingClientRect();
      const drop = d.started && d.y >= box.top - 24 && d.y <= box.bottom + 24;
      cancel();
      if (drop) reorder.current(d.key, d.before);
    };
    const pointerCancel = (event: PointerEvent) => {
      if (drag.current?.pointer === event.pointerId) cancel();
    };
    const lostCapture = (event: PointerEvent) => {
      if (event.target === node) pointerCancel(event);
    };
    const wheel = (event: WheelEvent) => {
      if (event.ctrlKey || node.scrollWidth <= node.clientWidth) return;
      const raw = Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY;
      const delta = raw * (event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? node.clientWidth : 1);
      if (!delta) return;
      // Non-passive: vertical mouse wheels must not scroll the chat/page.
      event.preventDefault();
      node.scrollLeft += delta;
      measure();
    };
    node.addEventListener("wheel", wheel, { passive: false });
    window.addEventListener("pointermove", move, { passive: false });
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", pointerCancel);
    node.addEventListener("lostpointercapture", lostCapture);
    window.addEventListener("blur", cancel);
    return () => {
      cancel();
      node.removeEventListener("wheel", wheel);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", pointerCancel);
      node.removeEventListener("lostpointercapture", lostCapture);
      window.removeEventListener("blur", cancel);
    };
  }, []);
  const navigate = (direction: number) => {
    const node = viewport.current;
    if (!node) return;
    node.scrollLeft += direction * Math.max(80, node.clientWidth * 0.8);
    measure();
  };
  const overflowing = edges.left || edges.right;
  return (
    <div className="terminal-panel__tab-strip">
      <div ref={viewport} className="terminal-panel__tabs" role="tablist" aria-label="终端会话"
        onScroll={measure}
        onPointerDownCapture={(event) => {
          if (drag.current || event.isPrimary === false) return;
          suppressClick.current = false;
          if (event.button !== 0 || !open) return;
          const target = (event.target as Element).closest<HTMLElement>('button[role="tab"]');
          const tab = target?.closest<HTMLElement>(TAB);
          if (!tab) return;
          drag.current = { pointer: event.pointerId, key: keyOf(tab),
            startX: event.clientX, startY: event.clientY, x: event.clientX, y: event.clientY,
            started: false, before: null };
        }}
        onClickCapture={(event) => {
          if (suppressClick.current && event.detail !== 0) { event.preventDefault(); event.stopPropagation(); }
        }}
        onContextMenuCapture={(event) => {
          if (drag.current?.started || suppressClick.current) { event.preventDefault(); event.stopPropagation(); }
        }}
        onDragStart={(event) => event.preventDefault()}>
        {children}
      </div>
      {overflowing && <div className="terminal-panel__tab-navigation" role="group" aria-label="滚动终端标签">
        <button type="button" className="terminal-panel__icon" aria-label="向左滚动终端标签"
          disabled={!edges.left} onClick={() => navigate(-1)}><ChevronLeft size={14} /></button>
        <button type="button" className="terminal-panel__icon" aria-label="向右滚动终端标签"
          disabled={!edges.right} onClick={() => navigate(1)}><ChevronRight size={14} /></button>
      </div>}
    </div>
  );
}
