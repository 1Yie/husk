import { memo, useEffect, useRef, useState } from "react";
import { ArrowLeft, ArrowRight, Columns2, Minus, Pencil, Plus, RotateCcw, Rows2, Settings2, SquareTerminal, Trash2, X } from "lucide-react";
import {
  ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { openSettings } from "@/lib/settings-bus";
import type { SplitDirection } from "./pane-layout";

const ITEM = "gap-2 text-xs cursor-pointer";
const DANGER = `${ITEM} text-red-600 dark:text-red-400 focus:text-red-600 dark:focus:text-red-400`;

export const TerminalTab = memo(function TerminalTab({
  id, groupKey, label, customName, cwd, selected, open, panes, ended,
  previous, next, first, last, canCreate, canColumns, canRows, canCloseOthers,
  canMoveLeft, canMoveRight, onMove,
  onSelect, onClose, onNew, onSplit, onRename, onCloseOthers, onCloseAll,
}: {
  id: string; groupKey: number; label: string; customName?: string; cwd?: string;
  selected: boolean; open: boolean; panes: number; ended?: "error" | "exited";
  previous: number; next: number; first: number; last: number;
  canCreate: boolean; canColumns: boolean; canRows: boolean; canCloseOthers: boolean;
  canMoveLeft: boolean; canMoveRight: boolean; onMove: (key: number, direction: -1 | 1) => void;
  onSelect: (key: number, focus?: boolean) => void; onClose: (key: number) => void; onNew: (key: number) => void;
  onSplit: (key: number, direction: SplitDirection) => void;
  onRename: (key: number, name: string) => void;
  onCloseOthers: (key: number) => void; onCloseAll: () => void;
}) {
  const menu = useRef<HTMLDivElement>(null);
  const menuOpen = useRef(false);
  const input = useRef<HTMLInputElement>(null);
  const focusFrame = useRef<number>();
  const editingRef = useRef(false);
  const openRef = useRef(open);
  openRef.current = open;
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const tabId = `${id}-tab-${groupKey}`;
  useEffect(() => {
    if (!open) {
      if (menuOpen.current)
        menu.current?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
      editingRef.current = false;
      setEditing(false);
    }
  }, [open]);
  useEffect(() => {
    if (editing) { input.current?.focus(); input.current?.select(); }
  }, [editing]);
  useEffect(() => () => {
    if (focusFrame.current !== undefined) cancelAnimationFrame(focusFrame.current);
  }, []);
  const finish = (save: boolean, restoreFocus = false) => {
    if (!editingRef.current) return;
    editingRef.current = false;
    if (save && openRef.current) onRename(groupKey, draft);
    const previous = input.current;
    setEditing(false);
    if (!restoreFocus) return;
    focusFrame.current = requestAnimationFrame(() => {
      focusFrame.current = undefined;
      const tab = document.getElementById(tabId);
      const active = document.activeElement;
      if (openRef.current && !menuOpen.current && !editingRef.current
        && (active === document.body || active === previous || active === tab)) tab?.focus();
    });
  };
  return (
    <ContextMenu onOpenChange={(value) => {
      menuOpen.current = value;
      if (value && focusFrame.current !== undefined) {
        cancelAnimationFrame(focusFrame.current);
        focusFrame.current = undefined;
      }
    }}>
      <ContextMenuTrigger asChild disabled={editing}>
        <div className="terminal-panel__tab" data-terminal-tab-key={groupKey} data-active={selected} data-ended={ended || undefined}>
          {editing ? (
            <input ref={input} id={tabId} aria-label="终端标签名称" className="terminal-panel__tab-rename"
              value={draft} onChange={(event) => setDraft(event.target.value)} onBlur={() => finish(true)}
              onKeyDown={(event) => {
                event.stopPropagation();
                if (event.nativeEvent.isComposing) return;
                if (event.key === "Enter") { event.preventDefault(); finish(true, true); }
                if (event.key === "Escape") { event.preventDefault(); finish(false, true); }
              }} />
          ) : (
            <button type="button" role="tab" id={tabId} aria-selected={selected}
              aria-controls={`${id}-view-${groupKey}`} tabIndex={selected ? 0 : -1}
              aria-keyshortcuts="Alt+ArrowLeft Alt+ArrowRight"
              title={customName ? `${customName}\n${cwd ?? ""}` : cwd} onClick={() => onSelect(groupKey)}
              onPointerDown={(event) => {
                // The strip owns primary dragging; do not arm Radix's touch
                // long-press context menu while a finger is sorting a tab.
                if (event.button === 0) event.stopPropagation();
              }}
              onAuxClick={(event) => { if (event.button === 1) onClose(groupKey); }}
              onKeyDown={(event) => {
                if (event.altKey && (event.key === "ArrowLeft" || event.key === "ArrowRight")) {
                  event.preventDefault();
                  onMove(groupKey, event.key === "ArrowLeft" ? -1 : 1);
                  return;
                }
                const target = event.key === "ArrowRight" ? next : event.key === "ArrowLeft" ? previous
                  : event.key === "Home" ? first : event.key === "End" ? last : undefined;
                if (target !== undefined) {
                  event.preventDefault();
                  onSelect(target, false);
                  document.getElementById(`${id}-tab-${target}`)?.focus();
                }
              }}>
              <SquareTerminal size={13} aria-hidden="true" />
              <span className="terminal-panel__tab-label">{label}</span>
              {panes > 1 && <span className="terminal-panel__tab-state">({panes})</span>}
              {ended && <span className="terminal-panel__tab-state">{ended === "error" ? "错误" : "已退出"}</span>}
            </button>
          )}
          <button type="button" className="terminal-panel__tab-close"
            aria-label={panes > 1 ? `关闭终端组 ${label}` : `关闭终端 ${label}`}
            title={panes > 1 ? "关闭此组全部终端" : "关闭终端并结束进程"}
            onClick={() => onClose(groupKey)}><X size={12} /></button>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent ref={menu} className="min-w-[160px]">
        <ContextMenuItem className={ITEM} disabled={!canCreate} onSelect={() => onNew(groupKey)}>
          <Plus className="h-3.5 w-3.5" />新建终端
        </ContextMenuItem>
        <ContextMenuItem className={ITEM} disabled={!canColumns} onSelect={() => onSplit(groupKey, "columns")}>
          <Columns2 className="h-3.5 w-3.5" />左右分屏
        </ContextMenuItem>
        <ContextMenuItem className={ITEM} disabled={!canRows} onSelect={() => onSplit(groupKey, "rows")}>
          <Rows2 className="h-3.5 w-3.5" />上下分屏
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem className={ITEM} disabled={!canMoveLeft} onSelect={() => onMove(groupKey, -1)}>
          <ArrowLeft className="h-3.5 w-3.5" />向左移动标签
        </ContextMenuItem>
        <ContextMenuItem className={ITEM} disabled={!canMoveRight} onSelect={() => onMove(groupKey, 1)}>
          <ArrowRight className="h-3.5 w-3.5" />向右移动标签
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem className={ITEM} onSelect={() => {
          editingRef.current = true; setDraft(label); setEditing(true);
        }}><Pencil className="h-3.5 w-3.5" />重命名</ContextMenuItem>
        {customName && (
          <ContextMenuItem className={ITEM} onSelect={() => onRename(groupKey, "")}>
            <RotateCcw className="h-3.5 w-3.5" />恢复默认名称
          </ContextMenuItem>
        )}
        <ContextMenuItem className={ITEM} onSelect={() => openSettings("terminal")}>
          <Settings2 className="h-3.5 w-3.5" />终端设置
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem className={DANGER} onSelect={() => onClose(groupKey)}>
          <X className="h-3.5 w-3.5" />{panes > 1 ? "关闭当前标签组" : "关闭当前终端"}
        </ContextMenuItem>
        <ContextMenuItem className={ITEM} disabled={!canCloseOthers} title="关闭其他标签并结束其中的终端进程" onSelect={() => onCloseOthers(groupKey)}>
          <Minus className="h-3.5 w-3.5" />关闭其他标签
        </ContextMenuItem>
        <ContextMenuItem className={DANGER} title="关闭所有标签、窗格并结束全部终端进程" onSelect={onCloseAll}>
          <Trash2 className="h-3.5 w-3.5" />关闭全部终端
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
});
