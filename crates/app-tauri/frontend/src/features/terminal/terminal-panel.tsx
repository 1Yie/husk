import { useCallback, useEffect, useLayoutEffect, useId, useRef, useState } from "react";
import { beginLayoutResize } from "@/lib/layout-resize";
import type { TerminalAppearance } from "./terminal-session";
import {
  isAppDark, resolveTerminalFont, resolveTerminalPalette,
  useTerminalPrefs, type TerminalPrefs,
} from "./preferences";
import { afterTerminalReveal } from "./reveal";
import { createTerminalResizePreview } from "./resize-preview";
import { TerminalWorkspace } from "./terminal-workspace";
import "./terminal.css";

export interface TerminalPanelProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  cwd?: string;
}
const MIN_HEIGHT = 120;

/** Prefs + the app's current theme, as the renderer takes them. */
function appearanceOf(prefs: TerminalPrefs): TerminalAppearance {
  const css = getComputedStyle(document.documentElement);
  const token = (name: string) =>
    css.getPropertyValue(name).trim() || undefined;
  const palette = resolveTerminalPalette(prefs.scheme, isAppDark(), token);
  return {
    theme: { ...palette, cursorAccent: palette.background },
    fontFamily: resolveTerminalFont(
      prefs.fontFamily,
      token("--husk-code-font"),
    ),
    fontSize: prefs.fontSize,
    lineHeight: prefs.lineHeight,
    cursorStyle: prefs.cursorStyle,
    cursorBlink: prefs.cursorBlink,
    scrollback: prefs.scrollback,
  };
}

/** Re-derive the appearance whenever 设置 or the app theme changes. */
function useTerminalAppearance(): TerminalAppearance {
  const prefs = useTerminalPrefs();
  const [themeEpoch, setThemeEpoch] = useState(0);
  useEffect(() => {
    // `applyAppearance` flips `.dark` and writes the colour/font vars inline.
    const observer = new MutationObserver(() => setThemeEpoch((n) => n + 1));
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class", "style"],
    });
    return () => observer.disconnect();
  }, []);
  const [appearance, setAppearance] = useState(() => appearanceOf(prefs));
  useEffect(() => {
    const next = appearanceOf(prefs);
    setAppearance((previous) =>
      JSON.stringify(previous) === JSON.stringify(next) ? previous : next,
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    prefs.scheme,
    prefs.fontFamily,
    prefs.fontSize,
    prefs.lineHeight,
    prefs.cursorStyle,
    prefs.cursorBlink,
    prefs.scrollback,
    themeEpoch,
  ]);
  return appearance;
}

/** Keep mounted: open controls visibility, not the lifetime of shells. */
export function TerminalPanel({ open, onOpenChange, cwd }: TerminalPanelProps) {
  // Shells start on the first open and live until their tab closes; closing
  // the last one ends the session, and the next open starts a fresh one.
  const [generation, setGeneration] = useState(open ? 1 : 0);
  const empty = useRef(false);
  const height = useTerminalPrefs((s) => s.panelHeight);
  const updatePrefs = useTerminalPrefs((s) => s.update);
  const [resizeEpoch, setResizeEpoch] = useState(0);
  const [maxHeight, setMaxHeight] = useState(560);
  const root = useRef<HTMLElement>(null);
  const separator = useRef<HTMLDivElement>(null);
  const drag = useRef<{ pointer: number; y: number; height: number; next: number; releaseLayout: () => void } | null>(
    null,
  );
  const id = useId();
  const minHeight = Math.min(MIN_HEIGHT, maxHeight);
  const clamp = useCallback(
    (value: number) =>
      Math.round(Math.min(maxHeight, Math.max(minHeight, value))),
    [maxHeight, minHeight],
  );
  const shownHeight = clamp(height);
  const targetHeight = useRef(shownHeight);
  targetHeight.current = shownHeight;
  const preview = useRef<ReturnType<typeof createTerminalResizePreview>>();
  if (!preview.current) {
    preview.current = createTerminalResizePreview((next) => {
      if (root.current) root.current.style.height = `${next}px`;
      separator.current?.setAttribute("aria-valuenow", String(next));
    });
  }
  const appearance = useTerminalAppearance();
  const [activated, setActivated] = useState(false);

  useLayoutEffect(() => {
    if (!root.current) return;
    if (!open) setActivated(false);
    const releaseLayout = beginLayoutResize();
    const cancel = afterTerminalReveal(
      root.current,
      () => drag.current ? null : open ? targetHeight.current : 0,
      () => {
        setActivated(open);
        if (!open && empty.current) {
          empty.current = false;
          setGeneration(0);
        }
        releaseLayout();
      },
    );
    return () => {
      cancel();
      releaseLayout();
    };
  }, [open, shownHeight]);

  useLayoutEffect(() => {
    if (open) {
      const restart = empty.current;
      empty.current = false;
      setGeneration((n) => restart ? n + 1 : n || 1);
    }
    else {
      const d = drag.current;
      drag.current = null;
      preview.current?.cancel();
      root.current?.removeAttribute("data-resizing");
      separator.current?.setAttribute("aria-valuenow", String(shownHeight));
      d?.releaseLayout();
    }
  }, [open]);
  useEffect(() => () => {
    preview.current?.cancel();
    const d = drag.current;
    drag.current = null;
    d?.releaseLayout();
  }, []);
  useEffect(() => {
    const parent = root.current?.parentElement;
    if (!parent) return;
    let frame: number | undefined;
    const measure = () => {
      if (frame !== undefined) return;
      frame = requestAnimationFrame(() => {
        frame = undefined;
        if (parent.clientHeight > 0)
          setMaxHeight(
            Math.max(
              0,
              Math.min(parent.clientHeight * 0.7, parent.clientHeight - 96),
            ),
          );
      });
    };
    const observer = new ResizeObserver(measure);
    observer.observe(parent);
    measure();
    return () => {
      observer.disconnect();
      if (frame !== undefined) cancelAnimationFrame(frame);
    };
  }, []);

  const endDrag = () => {
    const d = drag.current;
    if (!d) return;
    drag.current = null;
    const finalHeight = clamp(d.next);
    preview.current?.flush(finalHeight);
    // Commit the last pointer size while transitions are still disabled.
    // Otherwise a not-yet-painted final move can start a new 240ms ease.
    root.current?.getBoundingClientRect();
    updatePrefs({ panelHeight: finalHeight });
    setResizeEpoch((epoch) => epoch + 1);
    root.current?.removeAttribute("data-resizing");
    d.releaseLayout();
  };

  return (
    <section
      ref={root}
      className="terminal-panel"
      aria-label="终端面板"
      aria-hidden={!open}
      {...(open ? {} : { inert: "" })}
      data-open={open}
      style={{
        height: open ? shownHeight : 0,
        ["--terminal-bg" as string]: appearance.theme.background,
        ["--terminal-fg" as string]: appearance.theme.foreground,
      }}
    >
      <div className="terminal-panel__surface">
      <div
        ref={separator}
        className="terminal-panel__separator"
        role="separator"
        aria-label="调整终端高度"
        aria-orientation="horizontal"
        aria-controls={id}
        aria-valuemin={Math.round(minHeight)}
        aria-valuemax={Math.round(maxHeight)}
        aria-valuenow={shownHeight}
        tabIndex={open ? 0 : -1}
        onKeyDown={(event) => {
          const next =
            event.key === "ArrowUp"
              ? shownHeight + 20
              : event.key === "ArrowDown"
                ? shownHeight - 20
                : event.key === "Home"
                  ? minHeight
                  : event.key === "End"
                    ? maxHeight
                    : undefined;
          if (next !== undefined) {
            event.preventDefault();
            updatePrefs({ panelHeight: clamp(next) });
          }
        }}
        onPointerDown={(event) => {
          if (event.button !== 0 || !open || drag.current) return;
          event.preventDefault();
          const initialHeight = clamp(root.current?.getBoundingClientRect().height || shownHeight);
          event.currentTarget.focus();
          event.currentTarget.setPointerCapture(event.pointerId);
          drag.current = {
            pointer: event.pointerId,
            y: event.clientY,
            height: initialHeight,
            next: initialHeight,
            releaseLayout: beginLayoutResize(),
          };
          root.current?.setAttribute("data-resizing", "true");
          preview.current?.flush(initialHeight);
        }}
        onPointerMove={(event) => {
          if (drag.current?.pointer === event.pointerId) {
            drag.current.next = clamp(drag.current.height + drag.current.y - event.clientY);
            preview.current?.move(drag.current.next);
          }
        }}
        onPointerUp={(event) => {
          if (drag.current?.pointer !== event.pointerId) return;
          drag.current.next = clamp(drag.current.height + drag.current.y - event.clientY);
          endDrag();
          if (event.currentTarget.hasPointerCapture(event.pointerId))
            event.currentTarget.releasePointerCapture(event.pointerId);
        }}
        onPointerCancel={endDrag}
        onLostPointerCapture={() => {
          if (drag.current) endDrag();
        }}
        onDoubleClick={() =>
          updatePrefs({
            panelHeight: clamp(maxHeight) === shownHeight ? 280 : maxHeight,
          })
        }
      />
      {generation > 0 && (
        <TerminalWorkspace
          key={generation}
          id={id}
          open={open}
          activated={open && activated}
          resizeEpoch={resizeEpoch}
          cwd={cwd}
          appearance={appearance}
          onHide={() => onOpenChange(false)}
          onEmpty={() => {
            empty.current = true;
            onOpenChange(false);
          }}
        />
      )}
      </div>
    </section>
  );
}
