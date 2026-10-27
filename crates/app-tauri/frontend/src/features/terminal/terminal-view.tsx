import { memo, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { ClipboardPaste, Copy, Eraser, RotateCcw, TextSelect, X } from "lucide-react";
import {
  ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator,
  ContextMenuShortcut, ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { pasteClipboardText } from "@/lib/agent-ipc/index";
import { isMac, shortcutLabel } from "@/lib/platform";
import { isLayoutResizing, onLayoutResizeEnd } from "@/lib/layout-resize";
import { TerminalSession, type SessionState, type TerminalAppearance } from "./terminal-session";
import type { TerminalPane } from "./pane-layout";
import { queueTerminalWork } from "./render-queue";
import { Skeleton } from "@/components/ui/skeleton";

const COPY_KEYS = shortcutLabel(isMac ? ["mod", "c"] : ["mod", "shift", "c"]);
const PASTE_KEYS = shortcutLabel(isMac ? ["mod", "v"] : ["mod", "shift", "v"]);
const MENU_ITEM = "gap-2 text-xs cursor-pointer";
const MENU_DANGER = `${MENU_ITEM} text-red-600 dark:text-red-400 focus:text-red-600 dark:focus:text-red-400`;
const MENU_ICON = "h-3.5 w-3.5 shrink-0";

async function readClipboard(): Promise<string | null> {
  try {
    const result = await pasteClipboardText();
    if (result) return result.text;
  } catch { /* Fall back to the browser clipboard. */ }
  try { return (await navigator.clipboard?.readText()) ?? null; } catch { return null; }
}

export function paneLabel(pane: TerminalPane, state?: SessionState): string {
  const path = state?.cwd ?? pane.cwd;
  return state?.title || path?.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || path || "终端";
}

function statusText(state: SessionState): string {
  if (state.status === "opening") return "正在启动终端…";
  if (state.status === "error") return state.message ?? "终端出错";
  const detail = state.signal ? `信号 ${state.signal}` : state.code == null ? "" : `退出码 ${state.code}`;
  return `进程已退出${detail ? `（${detail}）` : ""}`;
}

function TerminalLoadingSkeleton() {
  return (
    <div className="terminal-panel__loading" role="status" aria-label="正在启动终端" aria-live="polite">
      <span className="sr-only">正在启动终端…</span>
      <div aria-hidden="true" className="flex max-w-[520px] flex-col gap-2 animate-pulse motion-reduce:animate-none">
        <div className="flex items-center gap-2">
          <Skeleton className="terminal-panel__skeleton-line w-3" />
          <Skeleton className="terminal-panel__skeleton-line w-36" />
        </div>
        <Skeleton className="terminal-panel__skeleton-line w-[62%]" />
        <Skeleton className="terminal-panel__skeleton-line w-[44%]" />
        <Skeleton className="terminal-panel__skeleton-line w-[70%]" />
      </div>
    </div>
  );
}

export const TerminalView = memo(function TerminalView({
  pane, id, groupKey, selected, visible, activated, focused, split,
  x, y, width, height, resizeEpoch, appearance, onState, onClose, onActivate,
}: {
  pane: TerminalPane;
  id: string;
  groupKey: number;
  selected: boolean;
  visible: boolean;
  activated: boolean;
  focused: boolean;
  split: boolean;
  x: number; y: number; width: number; height: number;
  resizeEpoch: number;
  appearance: TerminalAppearance;
  onState: (key: number, value: SessionState) => void;
  onClose: (key: number) => void;
  onActivate: (group: number, pane: number) => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const session = useRef<TerminalSession>();
  const notify = useRef(onState);
  notify.current = onState;
  const visibleRef = useRef(false);
  visibleRef.current = visible && activated;
  const focusedRef = useRef(focused);
  focusedRef.current = focused;
  const appearanceRef = useRef(appearance);
  appearanceRef.current = appearance;
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<SessionState>({ status: "opening", cwd: pane.cwd });
  const [hasSelection, setHasSelection] = useState(false);
  const contextMenu = useRef<HTMLDivElement>(null);
  const menuOpen = useRef(false);
  const focusOwner = useRef<Element | null>(null);
  const focusRequest = useRef({ visible: false, focused: false, activated: false });
  const allowOutsideInput = useRef(false);
  useLayoutEffect(() => {
    const previous = focusRequest.current;
    if (!visible || !focused) allowOutsideInput.current = false;
    else {
      if (!previous.visible || !previous.focused) focusOwner.current = document.activeElement;
      if (activated && !previous.activated) allowOutsideInput.current = true;
    }
    focusRequest.current = { visible, focused, activated };
  }, [visible, focused, activated]);
  const canAutoFocus = useCallback(() => {
    const element = host.current;
    if (!element || menuOpen.current) return false;
    const doc = element.ownerDocument;
    if (doc.querySelector('[role="menu"][data-state="open"]')) return false;
    const active = doc.activeElement;
    if (active && (element.contains(active) || active === element.parentElement)) return true;
    if (active instanceof HTMLElement && active.getAttribute("role") === "tab") return false;
    if (!allowOutsideInput.current && active instanceof HTMLElement
      && active.matches("input, textarea, [contenteditable=true], [role=textbox]")) return false;
    const owner = focusOwner.current;
    if (active !== owner && !(active === doc.body && owner && !owner.isConnected)) return false;
    const box = element.getBoundingClientRect();
    const hit = doc.elementFromPoint?.(box.left + box.width / 2, box.top + Math.min(20, box.height / 2));
    return !hit || !!element.parentElement?.contains(hit);
  }, []);
  const [started, setStarted] = useState(false);
  useEffect(() => {
    if (!activated || started) return;
    let frame: number | undefined;
    const schedule = () => {
      if (isLayoutResizing() || frame !== undefined) return;
      frame = requestAnimationFrame(() => {
        frame = requestAnimationFrame(() => {
          frame = undefined;
          if (!isLayoutResizing()) setStarted(true);
        });
      });
    };
    const unsubscribe = onLayoutResizeEnd(schedule);
    schedule();
    return () => {
      if (frame !== undefined) cancelAnimationFrame(frame);
      unsubscribe();
    };
  }, [activated, started]);

  useEffect(() => {
    if (!started || !host.current) return;
    const update = (value: SessionState) => {
      setState(value);
      notify.current(pane.key, value);
    };
    update({ status: "opening", cwd: pane.cwd });
    let dispose: (() => void) | undefined;
    const cancel = queueTerminalWork(() => {
      if (!host.current) return;
      let instance: TerminalSession;
      try {
        instance = new TerminalSession(host.current, pane.cwd, update, appearanceRef.current, readClipboard, canAutoFocus);
      } catch {
        update({ status: "error", cwd: pane.cwd, message: "无法初始化终端，请重试。" });
        return;
      }
      session.current = instance;
      if (visibleRef.current) instance.setVisible(true, focusedRef.current);
      else instance.setVisible(false);
      let observedWidth: number | undefined;
      const observer = new ResizeObserver((entries) => {
        const entry = entries[0];
        if (!entry) return;
        const width = entry.contentRect.width;
        if (!isLayoutResizing())
          instance.scheduleFit({ widthChanged: observedWidth !== undefined && width !== observedWidth });
        observedWidth = width;
      });
      observer.observe(host.current);
      let pendingFonts = false;
      const fontsChanged = () => {
        if (isLayoutResizing()) pendingFonts = true;
        else instance.remeasureFonts();
      };
      const unsubscribe = onLayoutResizeEnd(() => {
        if (pendingFonts) { pendingFonts = false; instance.remeasureFonts(); }
        instance.scheduleFit({ immediate: true });
      });
      document.fonts?.addEventListener?.("loadingdone", fontsChanged);
      dispose = () => {
        observer.disconnect();
        unsubscribe();
        document.fonts?.removeEventListener?.("loadingdone", fontsChanged);
        instance.dispose();
        session.current = undefined;
      };
    }, () => visibleRef.current && focusedRef.current);
    return () => { cancel(); dispose?.(); };
  }, [pane.key, pane.cwd, attempt, started, canAutoFocus]);

  useEffect(() => {
    // Hidden renderers keep their PTY/output, but don't need repeated font
    // measurements or palette rebuilds while the settings controls are used.
    // Catch up before setVisible schedules the next fit when selected again.
    if (visible && activated) session.current?.setAppearance(appearance);
  }, [appearance, visible, activated]);
  useEffect(() => {
    if (visible && activated) session.current?.setVisible(true, focused && !menuOpen.current);
    else session.current?.setVisible(false);
    if (!visible && menuOpen.current)
      contextMenu.current?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
  }, [visible, activated, focused]);
  useEffect(() => {
    if (!isLayoutResizing()) session.current?.scheduleFit({ immediate: true });
  }, [resizeEpoch]);

  const ended = state.status === "error" || state.status === "exited";
  const label = paneLabel(pane, state);
  return (
    <div
      role="region"
      tabIndex={-1}
      id={`${id}-pane-${pane.key}`}
      aria-labelledby={split ? `${id}-pane-label-${pane.key}` : `${id}-tab-${groupKey}`}
      className="terminal-panel__view"
      data-pane-key={pane.key}
      data-focused={focused}
      data-split={split || undefined}
      hidden={!selected}
      {...(visible ? {} : { inert: "" })}
      style={{ left: `${x * 100}%`, top: `${y * 100}%`, width: `${width * 100}%`, height: `${height * 100}%` }}
      onPointerDownCapture={() => onActivate(groupKey, pane.key)}
      onFocusCapture={(event) => {
        onActivate(groupKey, pane.key);
        if (event.target === event.currentTarget) session.current?.focus();
      }}
    >
      {split && (
        <div className="terminal-panel__pane-heading">
          <span id={`${id}-pane-label-${pane.key}`} className="terminal-panel__tab-label">{label}</span>
          <button type="button" className="terminal-panel__pane-close" aria-label={`关闭窗格 ${label}`}
            title="关闭此窗格并结束进程" onClick={() => onClose(pane.key)}><X size={12} /></button>
        </div>
      )}
      <ContextMenu onOpenChange={(isOpen) => {
        menuOpen.current = isOpen;
        if (isOpen) {
          session.current?.setVisible(visibleRef.current, false);
          setHasSelection(!!session.current?.hasSelection());
        }
      }}>
        <ContextMenuTrigger asChild><div ref={host} className="terminal-panel__screen" /></ContextMenuTrigger>
        <ContextMenuContent ref={contextMenu} className="min-w-[160px]">
          <ContextMenuItem className={MENU_ITEM} disabled={!hasSelection}
            onSelect={() => session.current?.copySelection()}>
            <Copy className={MENU_ICON} aria-hidden="true" />复制<ContextMenuShortcut>{COPY_KEYS}</ContextMenuShortcut>
          </ContextMenuItem>
          <ContextMenuItem className={MENU_ITEM} disabled={state.status !== "running"}
            onSelect={() => void session.current?.pasteFromClipboard()}>
            <ClipboardPaste className={MENU_ICON} aria-hidden="true" />粘贴<ContextMenuShortcut>{PASTE_KEYS}</ContextMenuShortcut>
          </ContextMenuItem>
          <ContextMenuItem className={MENU_ITEM} onSelect={() => session.current?.selectAll()}>
            <TextSelect className={MENU_ICON} aria-hidden="true" />全选
          </ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem className={MENU_ITEM} onSelect={() => {
            session.current?.clear(); session.current?.focus();
          }}>
            <Eraser className={MENU_ICON} aria-hidden="true" />清屏
          </ContextMenuItem>
          <ContextMenuItem className={MENU_ITEM} onSelect={() => setAttempt((value) => value + 1)}>
            <RotateCcw className={MENU_ICON} aria-hidden="true" />重新启动
          </ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem className={MENU_DANGER} onSelect={() => onClose(pane.key)}>
            <X className={MENU_ICON} aria-hidden="true" />关闭终端
          </ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>
      {state.status === "opening" && <TerminalLoadingSkeleton />}
      {ended && (
        <div className="terminal-panel__status" role="status" data-tone={state.status === "error" ? "error" : undefined}>
          <span>{statusText(state)}</span>
          <button type="button" onClick={() => setAttempt((value) => value + 1)}><RotateCcw size={12} />重新启动</button>
          <button type="button" onClick={() => onClose(pane.key)}>关闭</button>
        </div>
      )}
    </div>
  );
});
