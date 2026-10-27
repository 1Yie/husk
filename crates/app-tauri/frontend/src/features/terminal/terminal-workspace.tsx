import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Columns2, Minus, Plus, Rows2, Settings2 } from "lucide-react";
import { openSettings } from "@/lib/settings-bus";
import { TerminalVisibilityTooltip } from "@/components/layout-controls";
import { TerminalTabStrip } from "./terminal-tab-strip";
import { beginLayoutResize, isLayoutResizing, onLayoutResizeEnd } from "@/lib/layout-resize";
import { useTerminalPrefs } from "./preferences";
import type { SessionState, TerminalAppearance } from "./terminal-session";
import {
  layoutRectangles, listPanes, removePane, resizeSplit, splitPane,
  type PaneLayout, type SplitDirection,
} from "./pane-layout";
import { MIN_PANE_HEIGHT, MIN_PANE_WIDTH, SplitHandle } from "./split-handle";
import { paneLabel, TerminalView } from "./terminal-view";
import { TerminalTab } from "./terminal-tab";

const MAX_TERMINALS = 8;
type Group = { key: number; layout: PaneLayout; focused: number; name?: string };
type Workspace = { groups: Group[]; active: number; states: Record<number, SessionState> };
function countPanes(groups: Group[]) {
  return groups.reduce((sum, group) => sum + listPanes(group.layout).length, 0);
}
function hasSplitSpace(rect: { width: number; height: number } | undefined, bounds: { width: number; height: number }, direction: SplitDirection) {
  return !!rect
    && rect.width * bounds.width >= MIN_PANE_WIDTH * (direction === "columns" ? 2 : 1)
    && rect.height * bounds.height >= MIN_PANE_HEIGHT * (direction === "rows" ? 2 : 1);
}
function withoutGroup(state: Workspace, key: number): Workspace {
  const index = state.groups.findIndex((group) => group.key === key);
  if (index < 0) return state;
  const states = { ...state.states };
  for (const pane of listPanes(state.groups[index].layout)) delete states[pane.key];
  const groups = state.groups.filter((group) => group.key !== key);
  return { groups, states, active: state.active === key
    ? groups[Math.min(index, groups.length - 1)]?.key ?? 0 : state.active };
}
function withoutPane(state: Workspace, key: number): Workspace {
  const group = state.groups.find((group) => listPanes(group.layout).some((pane) => pane.key === key));
  if (!group) return state;
  const layout = removePane(group.layout, key);
  if (!layout) return withoutGroup(state, group.key);
  const focused = group.focused === key ? listPanes(layout)[0].key : group.focused;
  const states = { ...state.states };
  delete states[key];
  return { ...state, states, groups: state.groups.map((item) => item === group ? { ...group, layout, focused } : item) };
}

/** Suspend costly terminal fits until React has committed and painted new
 * pane rectangles. The views themselves remain mounted throughout. */
function useLayoutCommit() {
  const pending = useRef(new Set<{ frame: number; release: () => void }>());
  useEffect(() => () => {
    for (const item of pending.current) { cancelAnimationFrame(item.frame); item.release(); }
    pending.current.clear();
  }, []);
  return useCallback((update: () => void, after?: () => void) => {
    const ticket = { frame: 0, release: beginLayoutResize() };
    pending.current.add(ticket);
    update();
    ticket.frame = requestAnimationFrame(() => {
      ticket.frame = requestAnimationFrame(() => {
        pending.current.delete(ticket);
        ticket.release();
        after?.();
      });
    });
  }, []);
}

export function TerminalWorkspace({ id, open, activated, resizeEpoch, cwd, appearance, onHide, onEmpty }: {
  id: string; open: boolean; activated: boolean; resizeEpoch: number; cwd?: string;
  appearance: TerminalAppearance; onHide: () => void; onEmpty: () => void;
}) {
  const next = useRef(2);
  const [workspace, setWorkspace] = useState<Workspace>(() => ({
    groups: [{ key: 1, focused: 1, layout: { kind: "leaf", pane: { key: 1, cwd } } }], active: 1, states: {},
  }));
  const canvas = useRef<HTMLDivElement>(null);
  const commit = useLayoutCommit();
  const closeOnExit = useTerminalPrefs((state) => state.closeOnExit);
  const { groups, active, states } = workspace;
  const [tabReveal, setTabReveal] = useState<{ key: number }>();
  const reorder = useCallback((key: number, before: number | null) => {
    setWorkspace((previous) => {
      const source = previous.groups.find((group) => group.key === key);
      if (!source || key === before || (before !== null && !previous.groups.some((group) => group.key === before))) return previous;
      const groups = previous.groups.filter((group) => group !== source);
      const index = before === null ? groups.length : groups.findIndex((group) => group.key === before);
      groups.splice(index, 0, source);
      return groups.every((group, i) => group === previous.groups[i]) ? previous : { ...previous, groups };
    });
    setTabReveal({ key });
  }, []);
  const moveTab = useCallback((key: number, direction: -1 | 1) => {
    const index = groups.findIndex((group) => group.key === key);
    if (index < 0 || index + direction < 0 || index + direction >= groups.length) return;
    reorder(key, direction === -1 ? groups[index - 1].key : groups[index + 2]?.key ?? null);
  }, [groups, reorder]);
  const current = groups.find((group) => group.key === active);
  const focused = current && listPanes(current.layout).find((pane) => pane.key === current.focused);
  const currentCwd = focused && (states[focused.key]?.cwd ?? focused.cwd);
  const count = countPanes(groups);
  const [size, setSize] = useState({ width: 0, height: 0 });
  useLayoutEffect(() => {
    const node = canvas.current;
    if (!node) return;
    const measure = () => {
      if (isLayoutResizing()) return;
      const box = node.getBoundingClientRect();
      const width = Math.round(box.width), height = Math.round(box.height);
      setSize((old) => old.width === width && old.height === height ? old : { width, height });
    };
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    const unsubscribe = onLayoutResizeEnd(measure);
    measure();
    return () => { observer.disconnect(); unsubscribe(); };
  }, []);
  const focusedRect = current && layoutRectangles(current.layout).panes.find((rect) => rect.pane.key === current.focused);

  const activate = useCallback((groupKey: number, paneKey: number) => {
    setWorkspace((previous) => {
      const group = previous.groups.find((item) => item.key === groupKey);
      if (!group || !listPanes(group.layout).some((pane) => pane.key === paneKey)) return previous;
      if (previous.active === groupKey && group.focused === paneKey) return previous;
      return { ...previous, active: groupKey, groups: previous.groups.map((item) =>
        item === group ? { ...item, focused: paneKey } : item) };
    });
  }, []);
  const select = useCallback((key: number, focus = true) => {
    const owner = document.activeElement;
    commit(() => setWorkspace((previous) => previous.active === key ? previous : { ...previous, active: key }), () => {
      if (!focus || document.activeElement !== owner || document.querySelector('[role="menu"][data-state="open"]')) return;
      canvas.current?.querySelector<HTMLElement>(`[data-group-key="${key}"] [data-focused="true"]`)?.focus();
    });
  }, [commit]);
  const closePane = useCallback((key: number) => {
    commit(() => setWorkspace((previous) => withoutPane(previous, key)));
  }, [commit]);
  const closeGroup = useCallback((key: number) => {
    commit(() => setWorkspace((previous) => withoutGroup(previous, key)));
  }, [commit]);
  const add = useCallback((sourceKey?: number) => {
    if (!open) return;
    const key = next.current++;
    commit(() => setWorkspace((previous) => {
      if (countPanes(previous.groups) >= MAX_TERMINALS) return previous;
      const source = previous.groups.find((group) => group.key === sourceKey);
      const sourcePane = source && listPanes(source.layout).find((pane) => pane.key === source.focused);
      const directory = sourcePane ? previous.states[sourcePane.key]?.cwd ?? sourcePane.cwd : cwd;
      return { ...previous, active: key,
        groups: [...previous.groups, { key, focused: key, layout: { kind: "leaf", pane: { key, cwd: directory } } }] };
    }));
  }, [open, cwd, commit]);
  const split = useCallback((key: number, direction: SplitDirection) => {
    if (!open) return;
    const bounds = canvas.current?.getBoundingClientRect();
    if (!bounds) return;
    const paneKey = next.current++;
    commit(() => setWorkspace((previous) => {
      if (countPanes(previous.groups) >= MAX_TERMINALS) return previous;
      const target = previous.groups.find((group) => group.key === key);
      if (!target) return previous;
      const rect = layoutRectangles(target.layout).panes.find((item) => item.pane.key === target.focused);
      if (!rect || !hasSplitSpace(rect, bounds, direction)) return previous;
      const pane = { key: paneKey, cwd: previous.states[target.focused]?.cwd ?? rect.pane.cwd ?? cwd };
      const layout = splitPane(target.layout, target.focused, pane, direction);
      return { ...previous, active: key, groups: previous.groups.map((group) =>
        group === target ? { ...group, layout, focused: pane.key } : group) };
    }));
  }, [open, cwd, commit]);
  const rename = useCallback((key: number, value: string) => {
    const name = value.trim() || undefined;
    setWorkspace((previous) => {
      const target = previous.groups.find((group) => group.key === key);
      if (!target || target.name === name) return previous;
      return { ...previous, groups: previous.groups.map((group) => group === target ? { ...group, name } : group) };
    });
  }, []);
  const closeOthers = useCallback((key: number) => {
    commit(() => setWorkspace((previous) => {
      const group = previous.groups.find((group) => group.key === key);
      if (!group) return previous;
      const states: Workspace["states"] = {};
      for (const pane of listPanes(group.layout))
        if (previous.states[pane.key]) states[pane.key] = previous.states[pane.key];
      return { groups: [group], active: key, states };
    }));
  }, [commit]);
  const closeAll = useCallback(() => {
    commit(() => setWorkspace({ groups: [], active: 0, states: {} }));
  }, [commit]);
  const resize = useCallback((groupKey: number, splitKey: number, ratio: number) => {
    commit(() => setWorkspace((previous) => ({
      ...previous, groups: previous.groups.map((group) => {
        if (group.key !== groupKey) return group;
        const layout = resizeSplit(group.layout, splitKey, ratio);
        return layout === group.layout ? group : { ...group, layout };
      }),
    })));
  }, [commit]);
  const onState = useCallback((key: number, value: SessionState) => {
    if (closeOnExit && value.status === "exited" && value.code === 0 && !value.signal) {
      closePane(key);
      return;
    }
    setWorkspace((previous) => previous.groups.some((group) => listPanes(group.layout).some((pane) => pane.key === key))
      ? { ...previous, states: { ...previous.states, [key]: value } } : previous);
  }, [closeOnExit, closePane]);
  useEffect(() => { if (!groups.length) onEmpty(); }, [groups.length, onEmpty]);

  return (
    <div id={id} className="terminal-panel__inner">
      <div className="terminal-panel__toolbar">
        <TerminalTabStrip active={active} open={open} order={groups.map((group) => group.key).join(",")}
          reveal={tabReveal} onReorder={reorder}>
          {groups.map((group, index) => {
            const panes = listPanes(group.layout);
            const primary = panes.find((pane) => pane.key === group.focused) ?? panes[0];
            if (!primary) return null;
            const state = states[primary.key];
            const label = group.name ?? paneLabel(primary, state);
            const rect = layoutRectangles(group.layout).panes.find((item) => item.pane.key === primary.key);
            return (
              <TerminalTab key={group.key} id={id} groupKey={group.key} label={label} customName={group.name}
                cwd={state?.cwd ?? primary.cwd} selected={active === group.key} open={open} panes={panes.length}
                ended={state?.status === "error" || state?.status === "exited" ? state.status : undefined}
                previous={groups[(index + groups.length - 1) % groups.length].key}
                next={groups[(index + 1) % groups.length].key} first={groups[0].key} last={groups[groups.length - 1].key}
                canCreate={open && count < MAX_TERMINALS}
                canColumns={open && count < MAX_TERMINALS && hasSplitSpace(rect, size, "columns")}
                canRows={open && count < MAX_TERMINALS && hasSplitSpace(rect, size, "rows")}
                canCloseOthers={groups.length > 1} onSelect={select} onClose={closeGroup} onNew={add}
                canMoveLeft={index > 0} canMoveRight={index < groups.length - 1} onMove={moveTab}
                onSplit={split} onRename={rename} onCloseOthers={closeOthers} onCloseAll={closeAll} />
            );
          })}
        </TerminalTabStrip>
        <span className="terminal-panel__cwd" title={currentCwd}>{currentCwd}</span>
        <div className="terminal-panel__actions">
          <button type="button" className="terminal-panel__icon" aria-label="新建终端" disabled={!open || count >= MAX_TERMINALS}
            title={count >= MAX_TERMINALS ? "每个窗口最多 8 个终端" : "新建终端"} onClick={() => add()}><Plus size={15} /></button>
          <button type="button" className="terminal-panel__icon" aria-label="左右分屏"
            disabled={!open || count >= MAX_TERMINALS || !hasSplitSpace(focusedRect, size, "columns")}
            title={count >= MAX_TERMINALS ? "每个窗口最多 8 个终端" : "左右分屏"} onClick={() => split(active, "columns")}><Columns2 size={14} /></button>
          <button type="button" className="terminal-panel__icon" aria-label="上下分屏"
            disabled={!open || count >= MAX_TERMINALS || !hasSplitSpace(focusedRect, size, "rows")}
            title={count >= MAX_TERMINALS ? "每个窗口最多 8 个终端" : "上下分屏"} onClick={() => split(active, "rows")}><Rows2 size={14} /></button>
          <button type="button" className="terminal-panel__icon" aria-label="终端设置" title="终端设置" onClick={() => openSettings("terminal")}><Settings2 size={14} /></button>
          <TerminalVisibilityTooltip open>
            <button type="button" className="terminal-panel__icon" aria-label="隐藏终端面板" onClick={onHide}><Minus size={15} /></button>
          </TerminalVisibilityTooltip>
        </div>
      </div>
      <div ref={canvas} className="terminal-panel__workspace">
        {groups.map((group) => {
          const rectangles = layoutRectangles(group.layout);
          const selected = group.key === active;
          return (
            <div key={group.key} className="terminal-panel__group" role="tabpanel" id={`${id}-view-${group.key}`}
              data-group-key={group.key}
              aria-labelledby={`${id}-tab-${group.key}`} hidden={!selected}
              {...(open && selected ? {} : { inert: "" })}>
              {/* Flat, keyed siblings: splitting changes rectangles, never
                  the parent of an existing xterm/PTY host. */}
              {rectangles.panes.map(({ pane, x, y, width, height }) => (
                <TerminalView key={`pane-${pane.key}`} pane={pane} id={id} groupKey={group.key}
                  selected={selected} visible={open && selected} activated={activated}
                  focused={selected && group.focused === pane.key} split={rectangles.panes.length > 1}
                  x={x} y={y} width={width} height={height} resizeEpoch={resizeEpoch} appearance={appearance}
                  onState={onState} onClose={closePane} onActivate={activate} />
              ))}
              {rectangles.splits.map((split) => <SplitHandle key={`split-${split.key}`} split={split} layout={group.layout}
                groupKey={group.key} active={open && selected} onResize={resize} />)}
            </div>
          );
        })}
      </div>
    </div>
  );
}
