// App shell — sidebar on the left, `main-col` (title bar + stream +
// composer) on the right. The changes panel lives inside `main-col`,
// below the title bar, so it never covers the window controls.
import { cn } from "@/lib/utils";
import { isMac } from "@/lib/platform";
import { useEffect, useRef } from "react";
import { useSidebarGlass } from "./sidebar-glass";
import { useLayoutStore } from "../layout-store";
import { beginLayoutResize } from "@/lib/layout-resize";
import { useAppShortcuts } from "@/lib/use-app-shortcuts";

const SIDEBAR_DEFAULT_WIDTH = 240;
const SIDEBAR_MIN_WIDTH = 200;
const SIDEBAR_MAX_WIDTH = 480;

function installSidebarResize(shell: HTMLElement) {
  const navigation = shell.querySelector<HTMLElement>("#session-navigation")!;
  const handle = shell.querySelector<HTMLElement>(".session-navigation-resize")!;
  const readWidth = () => Number.parseFloat(shell.style.getPropertyValue("--session-sidebar-width")) || SIDEBAR_DEFAULT_WIDTH;
  const maximum = () => Math.min(SIDEBAR_MAX_WIDTH, shell.clientWidth > 0
    ? Math.max(SIDEBAR_MIN_WIDTH, shell.clientWidth - 480) : SIDEBAR_DEFAULT_WIDTH);
  const clamp = (width: number) => Math.round(Math.min(maximum(), Math.max(SIDEBAR_MIN_WIDTH, width)));
  let drag: { pointer: number; x: number; initial: number; next: number; release: () => void } | undefined;
  let frame = 0;
  const paint = (width: number) => {
    shell.style.setProperty("--session-sidebar-width", `${width}px`);
    handle.setAttribute("aria-valuenow", String(width));
    handle.setAttribute("aria-valuemax", String(maximum()));
  };
  const finish = (commit: boolean) => {
    const current = drag;
    if (!current) return;
    drag = undefined;
    cancelAnimationFrame(frame);
    frame = 0;
    paint(clamp(commit ? current.next : current.initial));
    navigation.getBoundingClientRect();
    navigation.removeAttribute("data-resizing");
    handle.removeAttribute("data-resizing");
    if (handle.hasPointerCapture(current.pointer)) handle.releasePointerCapture(current.pointer);
    current.release();
  };
  const down = (event: PointerEvent) => {
    if (event.button !== 0 || event.isPrimary === false || drag || !useLayoutStore.getState().sidebarOpen) return;
    event.preventDefault();
    handle.setPointerCapture(event.pointerId);
    const initial = clamp(readWidth());
    drag = { pointer: event.pointerId, x: event.clientX, initial, next: initial, release: beginLayoutResize() };
    navigation.setAttribute("data-resizing", "");
    handle.setAttribute("data-resizing", "");
  };
  const move = (event: PointerEvent) => {
    if (!drag || drag.pointer !== event.pointerId) return;
    drag.next = clamp(drag.initial + event.clientX - drag.x);
    if (!frame) frame = requestAnimationFrame(() => {
      frame = 0;
      if (drag) paint(drag.next);
    });
  };
  const up = (event: PointerEvent) => {
    if (!drag || drag.pointer !== event.pointerId) return;
    drag.next = clamp(drag.initial + event.clientX - drag.x);
    finish(true);
  };
  const cancel = () => finish(false);
  const key = (event: KeyboardEvent) => {
    if (event.key === "Escape" && drag) {
      event.preventDefault();
      cancel();
      return;
    }
    if (event.target !== handle || !useLayoutStore.getState().sidebarOpen) return;
    const next = event.key === "ArrowLeft" ? readWidth() - 24
      : event.key === "ArrowRight" ? readWidth() + 24
      : event.key === "Home" ? SIDEBAR_MIN_WIDTH : event.key === "End" ? maximum() : undefined;
    if (next === undefined) return;
    event.preventDefault();
    const release = beginLayoutResize();
    navigation.setAttribute("data-resizing", "");
    paint(clamp(next));
    navigation.getBoundingClientRect();
    navigation.removeAttribute("data-resizing");
    release();
  };
  const measure = () => {
    if (drag) {
      drag.next = clamp(drag.next);
      paint(drag.next);
    } else if (readWidth() > maximum()) {
      const release = beginLayoutResize();
      navigation.setAttribute("data-resizing", "");
      paint(clamp(readWidth()));
      navigation.getBoundingClientRect();
      navigation.removeAttribute("data-resizing");
      release();
    } else handle.setAttribute("aria-valuemax", String(maximum()));
  };
  paint(clamp(readWidth()));
  const observer = new ResizeObserver(measure);
  observer.observe(shell);
  const unsubscribe = useLayoutStore.subscribe((state) => {
    if (!state.sidebarOpen) cancel();
  });
  handle.addEventListener("pointerdown", down);
  handle.addEventListener("lostpointercapture", cancel);
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", cancel);
  window.addEventListener("blur", cancel);
  window.addEventListener("keydown", key, true);
  return () => {
    cancel();
    observer.disconnect();
    unsubscribe();
    handle.removeEventListener("pointerdown", down);
    handle.removeEventListener("lostpointercapture", cancel);
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", cancel);
    window.removeEventListener("blur", cancel);
    window.removeEventListener("keydown", key, true);
  };
}

interface MainLayoutProps {
  sidebar: React.ReactNode;
  children: React.ReactNode;
  terminal?: React.ReactNode;
  className?: string;
}

export function MainLayout({ sidebar, children, terminal, className }: MainLayoutProps) {
  const shell = useRef<HTMLDivElement>(null);
  useSidebarGlass(shell);
  useAppShortcuts();
  const sidebarOpen = useLayoutStore((state) => state.sidebarOpen);
  useEffect(() => {
    const disposeResize = shell.current ? installSidebarResize(shell.current) : () => {};
    return () => disposeResize();
  }, []);
  const main = (
    <div className={cn("main-col isolate h-full overflow-hidden",
      isMac && "bg-transparent")}>{children}{terminal}</div>
  );
  return (
    <div
      ref={shell}
      className={cn(
        "main-layout flex h-full w-full bg-white overflow-hidden select-none",
        className
      )}
    >
      <div id="session-navigation" aria-hidden={!sidebarOpen}
        {...(!sidebarOpen ? { inert: "" } : {})}
        className="h-full flex-none overflow-hidden transition-[width] duration-300 ease-out motion-reduce:transition-none"
        style={{ width: sidebarOpen ? `var(--session-sidebar-width, ${SIDEBAR_DEFAULT_WIDTH}px)` : 0 }}>
        <div style={{ width: `var(--session-sidebar-width, ${SIDEBAR_DEFAULT_WIDTH}px)` }} className={cn("h-full transition-transform duration-300 ease-out motion-reduce:transition-none",
          sidebarOpen ? "translate-x-0" : "-translate-x-full")}>
          {sidebar}
        </div>
      </div>
      <div className={cn("relative flex h-full min-w-0 flex-1",
        isMac && "main-workspace-frame bg-panel px-2 pb-2")}>
        {main}
        <div className={cn("session-navigation-resize layout-resize-grip absolute bottom-0 top-9 z-50 w-1.5 cursor-col-resize",
          isMac ? "left-px" : "-left-[3px]")}
          role="separator" aria-label="调整会话侧栏宽度" aria-orientation="vertical"
          aria-controls="session-navigation" aria-valuemin={SIDEBAR_MIN_WIDTH} aria-valuemax={SIDEBAR_MAX_WIDTH} aria-valuenow={SIDEBAR_DEFAULT_WIDTH}
          hidden={!sidebarOpen} tabIndex={sidebarOpen ? 0 : -1} />
      </div>
    </div>
  );
}
