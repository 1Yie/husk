import { SquarePen, Search, Clock, Plug, Folder, Settings, MoreHorizontal, GitFork, Bin } from "@keyline-icons/react";
import { useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { SessionRow } from "../../types";
import { Orb } from "../agent-orb";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

const win = getCurrentWindow();

interface Props {
  sessions: SessionRow[];
  activeId?: number;
  workspaceName?: string;
  workspaceRoot?: string;
  recentWorkspaces?: { path: string; name: string; last_opened: number }[];
  onNew: () => void;
  onOpen: (id: number) => void;
  onPickWorkspace?: () => void;
  onSwitchWorkspace?: (path: string) => void;
  onOpenSettings?: () => void;
  onFork?: (id: number) => void;
  onDelete?: (id: number) => void;
}

export function SessionSidebar({
  sessions,
  activeId,
  workspaceName,
  workspaceRoot,
  recentWorkspaces = [],
  onNew,
  onOpen,
  onPickWorkspace,
  onSwitchWorkspace,
  onOpenSettings,
  onFork,
  onDelete,
}: Props) {
  const otherRecents = recentWorkspaces.filter((w) => w.path !== workspaceRoot);

  const drag = (e: React.MouseEvent) => {
    const el = e.target as HTMLElement;
    if (e.button === 0 && el.closest("[data-drag]") && !el.closest("[data-nodrag]")) {
      void win.startDragging();
    }
  };

  const doubleClick = (e: React.MouseEvent) => {
    const el = e.target as HTMLElement;
    if (el.closest("[data-drag]") && !el.closest("[data-nodrag]")) {
      void win.toggleMaximize();
    }
  };

  return (
    <aside className="w-[230px] flex-none flex flex-col bg-neutral-100/70 border-r border-neutral-200 select-none h-full">
      {/* Top draggable header aligning with main content topbar */}
      <div
        onMouseDown={drag}
        onDoubleClick={doubleClick}
        data-drag
        data-tauri-drag-region
        className="h-9 flex-none flex items-center px-3 gap-2 border-b border-neutral-200/80 bg-neutral-100/80 select-none cursor-default"
      >
        <span className="w-2.5 h-2.5 rounded-full bg-neutral-400 inline-block shrink-0" />
        <span className="text-[12px] font-semibold text-neutral-700 tracking-tight">
          agent-rs
        </span>
      </div>

      {/* Active Workspace Card */}
      <div className="p-2 pb-1">
        <div className="p-2 rounded-md bg-white border border-neutral-200/80 shadow-xs flex flex-col gap-1.5">
          <div className="flex items-center justify-between gap-1.5">
            <div className="flex items-center gap-1.5 min-w-0">
              <Folder className="h-4 w-4 text-neutral-600 flex-none" />
              <span className="text-[13px] font-semibold text-neutral-900 truncate" title={workspaceRoot}>
                {workspaceName || "工作区"}
              </span>
            </div>
            <Button
              variant="outline"
              size="sm"
              onClick={onPickWorkspace}
              className="h-6 px-2 text-[11px] font-medium text-neutral-600 border-neutral-200 hover:bg-neutral-100"
            >
              打开
            </Button>
          </div>
          <div className="text-[10px] text-neutral-400 truncate" title={workspaceRoot}>
            {workspaceRoot || "未选定目录"}
          </div>
        </div>
      </div>

      {/* New Session Button inside workspace */}
      <div className="px-2 pt-1 pb-1">
        <Button
          variant="secondary"
          onClick={onNew}
          className="w-full justify-start gap-2 px-2.5 h-8 text-[13px] font-medium bg-neutral-900 text-white hover:bg-neutral-800"
        >
          <SquarePen className="h-4 w-4 text-white" />
          <span>新建会话</span>
        </Button>
      </div>

      {/* Section title */}
      <div className="px-3 pt-3 pb-1 flex items-center justify-between">
        <span className="text-[11px] font-medium uppercase tracking-wider text-neutral-400">
          会话 ({sessions.length})
        </span>
      </div>

      {/* Sessions list — native scroller (no ScrollArea). `min-h-0` lets
          the flex child shrink and scroll instead of growing past the
          aside's height; the native scrollbar takes real layout space so
          nothing sits under it. */}
      {/* `overflow-y-scroll` keeps the gutter reserved permanently, so the
          scrollbar appearing/disappearing never shifts row content.
          `transform-gpu` (translateZ(0)) promotes the scroller to a
          composited layer — under WebKitGTK an uncomposited scroller paints
          the custom scrollbar in the same layer as its contents, so any
          row repaint (e.g. a hover background) repaints the thumb too and
          it visibly flashes. Composited scrollers get their own scrollbar
          layers, so content repaints can't touch it. */}
      <div className="flex-1 min-h-0 overflow-y-scroll transform-gpu">
        <div className="flex flex-col gap-0.5 px-2 pb-2">
          {sessions.length === 0 ? (
            <div className="px-2.5 py-1.5 text-[13px] text-neutral-400">暂无会话</div>
          ) : (
            sessions.map((s) => (
              <SessionItem
                key={s.id}
                row={s}
                isActive={activeId ? s.id === activeId : s.active}
                onOpen={onOpen}
                onFork={onFork}
                onDelete={onDelete}
              />
            ))
          )}
        </div>
      </div>

      {/* Recent workspaces footer */}
      {otherRecents.length > 0 && onSwitchWorkspace && (
        <div className="p-2 border-t border-neutral-200/60 flex flex-col gap-1">
          <span className="px-1 text-[10px] font-medium uppercase tracking-wider text-neutral-400">
            最近工作区
          </span>
          <div className="flex flex-col gap-0.5 max-h-[100px] overflow-y-auto">
            {otherRecents.slice(0, 4).map((w) => (
              <Button
                key={w.path}
                variant="ghost"
                onClick={() => onSwitchWorkspace(w.path)}
                className="w-full justify-start gap-2 px-2 h-7 text-[12px] text-neutral-600 hover:text-neutral-900 hover:bg-neutral-200/60"
                title={w.path}
              >
                <Folder className="h-3.5 w-3.5 text-neutral-400 flex-none" />
                <span className="truncate">{w.name}</span>
              </Button>
            ))}
          </div>
        </div>
      )}

      {/* Settings footer - seamless border-t without side gaps */}
      <div className="p-2 border-t border-neutral-200/80">
        <Button
          variant="ghost"
          onClick={onOpenSettings}
          className="w-full justify-start gap-2.5 px-2.5 h-8 text-[13px] font-normal text-neutral-700 hover:bg-neutral-200/60"
        >
          <Settings className="h-4 w-4 text-neutral-500" />
          设置
        </Button>
      </div>
    </aside>
  );
}

function SidebarNavItem({
  icon,
  label,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  onClick?: () => void;
  highlight?: boolean;
}) {
  return (
    <Button
      variant="ghost"
      onClick={onClick}
      className="justify-start gap-2.5 px-2.5 h-8 text-[13px] font-normal text-neutral-700 hover:bg-neutral-200/60"
    >
      <span className="text-neutral-500 [&_svg]:h-4 [&_svg]:w-4">{icon}</span>
      {label}
    </Button>
  );
}

function SessionItem({
  row,
  isActive,
  onOpen,
  onFork,
  onDelete,
}: {
  row: SessionRow;
  isActive: boolean;
  onOpen: (id: number) => void;
  onFork?: (id: number) => void;
  onDelete?: (id: number) => void;
}) {
  const displayTitle =
    row.title === "new session" ? "新会话" : (row.title || `对话 ${row.id}`);
  const [menuOpen, setMenuOpen] = useState(false);

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => onOpen(row.id)}
      onKeyDown={(e) => { if (e.key === "Enter") onOpen(row.id); }}
      className={cn(
        "group flex w-full items-center gap-2 rounded-md px-2.5 h-8 text-[13px] cursor-pointer select-none",
        isActive
          ? "bg-white text-neutral-900 font-medium shadow-sm border border-neutral-200/80"
          : "text-neutral-600 hover:bg-neutral-200/50 font-normal"
      )}
    >
      <span className="flex-1 min-w-0 truncate text-left">
        {displayTitle}
      </span>
      {/* Trailing cell — the running orb and the ⋯ trigger share one 20px
          slot: the orb shows at rest and swaps to the trigger on row
          hover/focus, so "running" never costs a second cell of title
          width. While the menu is open the orb is unmounted entirely —
          `group-hover` alone would let it peek back out once the pointer
          leaves the row for the (portaled) menu. */}
      <div className="relative flex-none h-5 w-5">
        {row.running && !menuOpen && (
          <Orb
            variant="C3"
            size={14}
            className="absolute inset-0 flex items-center justify-center text-neutral-800 group-hover:invisible group-focus-within:invisible"
          />
        )}
        {/* modal={false} — a modal menu makes Radix scroll-lock the <body>
            (strips the scrollbar + adds padding compensation), which is the
            weird scrollbar vanish/reappear + layout shift on open/close. */}
        {/* Trigger uses `invisible`/`visible`, not an opacity fade: an
            opacity transition promotes the button to a composited layer for
            the fade and demotes it after, and each promotion repaints the
            scroller — that repaint is what flashed the native scrollbar on
            every row hover. `visibility` never touches the compositor, and
            the hidden button also stops being hit-testable (an opacity-0
            button still swallowed clicks on the row's right edge).
            `group-focus-within` keeps it keyboard-reachable. */}
        <DropdownMenu modal={false} onOpenChange={setMenuOpen}>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              aria-label="会话操作"
              onClick={(e) => e.stopPropagation()}
              onKeyDown={(e) => e.stopPropagation()}
              className="absolute inset-0 rounded flex items-center justify-center text-neutral-400 invisible group-hover:visible group-focus-within:visible data-[state=open]:visible hover:bg-neutral-300/60 hover:text-neutral-700"
            >
              <MoreHorizontal className="h-3.5 w-3.5" />
            </button>
          </DropdownMenuTrigger>
        <DropdownMenuContent side="right" align="start" sideOffset={4} className="min-w-[140px]">
          <DropdownMenuItem
            className="gap-2 text-xs cursor-pointer"
            onClick={(e) => { e.stopPropagation(); onFork?.(row.id); }}
          >
            <GitFork className="h-3.5 w-3.5" />
            Fork 会话
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            className="gap-2 text-xs cursor-pointer text-red-600 focus:text-red-600 focus:bg-red-50 dark:focus:bg-red-950/40"
            onClick={(e) => { e.stopPropagation(); onDelete?.(row.id); }}
          >
            <Bin className="h-3.5 w-3.5" />
            删除会话
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      </div>
    </div>
  );
}
