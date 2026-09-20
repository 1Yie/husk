import {
  Plus,
  ChevronRight,
  ChevronDown,
  Folder,
  FolderOpen,
  Settings,
  Bell,
  MoreHorizontal,
  GitFork,
  Bin,
  Bookmark,
} from "@keyline-icons/react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { ProjectOverview, SessionRow } from "../../types";
import { Orb } from "../agent-orb";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
  TooltipSimple,
} from "@/components/ui/tooltip";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { isMac } from "@/lib/platform";
import { cn } from "@/lib/utils";

/** Cross-project recent list cap — everything beyond it stays reachable by
 * opening the owning project in 项目. */
const RECENT_LIMIT = 6;

interface Props {
  /** Project tree — active workspace first, then recent ones, each with its
   * full conversation list (`SessionManager::projects_overview`). */
  projects: ProjectOverview[];
  activeId?: number;
  version?: string;
  onNew: () => void;
  /** Open a conversation anywhere in the tree — `root` lets the shell switch
   * workspaces when the row belongs to another project. */
  onOpenSession: (root: string, id: number) => void;
  onPickWorkspace?: () => void;
  onSwitchWorkspace?: (path: string) => void;
  onOpenSettings?: () => void;
  onFork?: (id: number) => void;
  onPin?: (id: number, pinned: boolean) => void;
  onDelete?: (id: number) => void;
}

export function SessionSidebar({
  projects,
  activeId,
  version: propVersion,
  onNew,
  onOpenSession,
  onPickWorkspace,
  onSwitchWorkspace,
  onOpenSettings,
  onFork,
  onPin,
  onDelete,
}: Props) {
  // Global recents: every project's conversations merged — pinned first
  // (each tier newest-first), so the top of the sidebar always answers
  // "what was I just doing" while pinned sessions stay reachable.
  const recents = useMemo(() => {
    const all = projects.flatMap((p) =>
      p.sessions.map((s) => ({
        root: p.root,
        project: p.name,
        current: p.current,
        row: s,
      })),
    );
    all.sort((a, b) =>
      Number(b.row.pinned) - Number(a.row.pinned) ||
      b.row.updated_at - a.row.updated_at
    );
    return all.slice(0, RECENT_LIMIT);
  }, [projects]);

  // Projects open/close independently — opening one reveals its full
  // conversation list without switching the active workspace. The current
  // workspace's project starts expanded by default.
  const currentRoot = projects.find((p) => p.current)?.root;
  const [expanded, setExpanded] = useState<Set<string>>(() =>
    currentRoot ? new Set([currentRoot]) : new Set()
  );
  // When the current workspace changes (first tree load lands async, or a
  // switch lands), its project opens by default. A manual collapse still
  // wins — this only fires when `currentRoot` itself changes.
  const lastCurrentRef = useRef(currentRoot);
  useEffect(() => {
    if (!currentRoot || currentRoot === lastCurrentRef.current) return;
    lastCurrentRef.current = currentRoot;
    setExpanded((prev) => {
      if (prev.has(currentRoot)) return prev;
      const next = new Set(prev);
      next.add(currentRoot);
      return next;
    });
  }, [currentRoot]);
  const toggleProject = (root: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(root)) next.delete(root);
      else next.add(root);
      return next;
    });
  };

  return (
    <TooltipProvider delayDuration={300}>
      <aside
        data-tauri-drag-region="deep"
        className="w-[240px] flex-none flex flex-col bg-panel border-r border-hairline select-none h-full text-neutral-800"
      >
        <div
          data-tauri-drag-region="deep"
          className="h-9 flex-none flex items-center px-3 gap-2 border-b border-hairline bg-panel select-none cursor-default"
        >
          {isMac && <div className="w-[78px] shrink-0" />}

          <span className="text-[12px] font-semibold text-neutral-700 tracking-tight">
            Husk
          </span>
        </div>

        <div className="flex-1 min-h-0 flex flex-col">
          <div className="flex-none flex items-center justify-between px-4 pt-3 pb-1">
            <span className="text-[13px] font-medium text-neutral-600">
              会话
            </span>
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  data-nodrag
                  data-tauri-drag-region="false"
                  onClick={onNew}
                  aria-label="新建会话"
                  className="h-5 w-5 rounded flex items-center justify-center text-neutral-400 hover:text-neutral-700 hover:bg-neutral-200/60 transition-colors"
                >
                  <Plus className="h-3.5 w-3.5" />
                </button>
              </TooltipTrigger>
              <TooltipContent side="right" className="text-xs">
                新建会话
              </TooltipContent>
            </Tooltip>
          </div>

          {/* Recent sessions — newest first across every project, capped so
              the full archive stays inside 项目. */}
          <div className="flex-none flex flex-col gap-0.5 px-2 pb-1">
            {recents.length === 0 ? (
              <div className="pl-7 h-8 flex items-center shrink-0 flex-none text-[13px] text-neutral-400">
                暂无会话
              </div>
            ) : (
              recents.map(({ root, project, current, row }) => (
                <SessionItem
                  key={`${root}:${row.id}`}
                  row={row}
                  hint={current ? undefined : project}
                  isActive={current && (activeId ? row.id === activeId : row.active)}
                  showActions={current}
                  onOpen={() => onOpenSession(root, row.id)}
                  onFork={onFork}
                  onPin={onPin}
                  onDelete={onDelete}
                />
              ))
            )}
          </div>

          {/* Section: 项目 — open a project to read its full conversation list */}
          <div className="flex-none flex items-center justify-between px-4 pt-2 pb-1">
            <span className="text-[13px] font-medium text-neutral-600">
              项目
            </span>
          </div>

          {/* Project list: the content area's only scroller. */}
          <div className="flex-1 min-h-0 overflow-y-auto transform-gpu flex flex-col gap-0.5 px-2 pb-2">
            {projects.length === 0 ? (
              <div className="pl-3 h-8 flex items-center text-[13px] text-neutral-400">
                暂无项目
              </div>
            ) : (
              projects.map((p) => (
                <ProjectItem
                  key={p.root}
                  project={p}
                  open={expanded.has(p.root)}
                  activeId={activeId}
                  onToggle={() => toggleProject(p.root)}
                  onOpenSession={onOpenSession}
                  onSwitchWorkspace={onSwitchWorkspace}
                  onFork={onFork}
                  onPin={onPin}
                  onDelete={onDelete}
                />
              ))
            )}

            {onPickWorkspace && (
              <button
                type="button"
                data-nodrag
                data-tauri-drag-region="false"
                onClick={onPickWorkspace}
                className="flex items-center gap-1.5 px-2 py-1.5 rounded-md text-[13px] text-neutral-600 hover:text-neutral-900 hover:bg-black/[0.04] transition-colors"
              >
                <FolderOpen className="h-3.5 w-3.5 shrink-0 text-neutral-400" />
                <span>打开其他工作区...</span>
              </button>
            )}
          </div>
        </div>

        <div className="h-11 px-4 flex items-center select-none flex-none bg-panel">
          <div data-nodrag data-tauri-drag-region="false" className="flex items-center gap-3.5">
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  onClick={onOpenSettings}
                  aria-label="设置"
                  className="text-neutral-500 hover:text-neutral-800 transition-colors p-1 rounded hover:bg-black/[0.04]"
                >
                  <Settings className="h-4 w-4" />
                </button>
              </TooltipTrigger>
              <TooltipContent side="top" className="text-xs">
                设置
              </TooltipContent>
            </Tooltip>

            <Popover>
              <Tooltip>
                <TooltipTrigger asChild>
                  <PopoverTrigger asChild>
                    <button
                      type="button"
                      aria-label="通知"
                      className="text-neutral-500 hover:text-neutral-800 transition-colors p-1 rounded hover:bg-black/[0.04]"
                    >
                      <Bell className="h-4 w-4" />
                    </button>
                  </PopoverTrigger>
                </TooltipTrigger>
                <TooltipContent side="top" className="text-xs">
                  通知
                </TooltipContent>
              </Tooltip>
              <PopoverContent side="top" align="start" className="w-56 p-3 text-xs">
                <div className="font-semibold text-neutral-800 mb-1">
                  系统通知
                </div>
                <div className="text-neutral-500">暂无新通知。</div>
              </PopoverContent>
            </Popover>
          </div>
        </div>
      </aside>
    </TooltipProvider>
  );
}

function SessionItem({
  row,
  isActive,
  hint,
  showActions = true,
  onOpen,
  onFork,
  onPin,
  onDelete,
}: {
  row: SessionRow;
  isActive: boolean;
  /** Trailing project label — set on the cross-project recent list so rows
   * from different projects stay tellable apart. */
  hint?: string;
  /** Fork/delete only exist for the active workspace: its store and actors
   * are the only ones the kernel can address (ids are per-workspace). */
  showActions?: boolean;
  onOpen: () => void;
  onFork?: (id: number) => void;
  onPin?: (id: number, pinned: boolean) => void;
  onDelete?: (id: number) => void;
}) {
  const displayTitle = row.title === "new session" ? "新会话" : row.title || `会话 ${row.id}`;
  const [menuOpen, setMenuOpen] = useState(false);

  // Identical items for the "···" dropdown and the row's right-click menu.
  // The two Radix families keep separate `Menu` contexts, so the same JSX
  // can't be shared — one builder renders the items with whichever
  // Item/Separator primitives the host menu provides.
  const renderActionItems = (
    Item: typeof DropdownMenuItem,
    Separator: typeof DropdownMenuSeparator,
  ) => (
    <>
      <Item
        className="gap-2 text-xs cursor-pointer"
        onClick={(e) => {
          e.stopPropagation();
          onPin?.(row.id, !row.pinned);
        }}
      >
        <Bookmark className={cn("h-3.5 w-3.5", row.pinned && "fill-current")} />
        {row.pinned ? "取消置顶" : "置顶会话"}
      </Item>
      <Item
        className="gap-2 text-xs cursor-pointer"
        onClick={(e) => {
          e.stopPropagation();
          onFork?.(row.id);
        }}
      >
        <GitFork className="h-3.5 w-3.5" />
        Fork 会话
      </Item>
      <Separator />
      <Item
        className="gap-2 text-xs cursor-pointer text-red-600 dark:text-red-400 focus:text-red-600 dark:focus:text-red-400 focus:bg-red-50 dark:focus:bg-red-950/40"
        onClick={(e) => {
          e.stopPropagation();
          onDelete?.(row.id);
        }}
      >
        <Bin className="h-3.5 w-3.5" />
        删除会话
      </Item>
    </>
  );

  const rowEl = (
    <div
      role="button"
      tabIndex={0}
      data-nodrag
      data-tauri-drag-region="false"
      onClick={onOpen}
      onKeyDown={(e) => {
        if (e.key === "Enter") onOpen();
      }}
      className={cn(
        "group flex-none shrink-0 flex w-full items-center gap-1.5 rounded-md pl-7 pr-2 h-8 min-h-8 text-[13px] cursor-pointer select-none transition-colors",
        isActive
          ? "bg-black/[0.06] text-neutral-900 font-medium"
          : "text-neutral-600 hover:bg-black/[0.04] hover:text-neutral-900 font-normal",
      )}
    >
      <span className="flex-1 min-w-0 truncate text-left">{displayTitle}</span>
      {row.pinned && (
        <Bookmark className="h-3 w-3 flex-none fill-current text-amber-500 dark:text-amber-400" />
      )}
      {hint && (
        <span className="flex-none max-w-[64px] truncate text-[11px] text-neutral-400">
          {hint}
        </span>
      )}
      {/* Trailing cell: orb at rest, more menu on hover/focus */}
      <div className="relative flex-none h-5 w-5">
        {row.running && !menuOpen && (
          <Orb
            variant="C3"
            size={14}
            className="absolute inset-0 flex items-center justify-center text-neutral-800 group-hover:invisible group-focus-within:invisible"
          />
        )}
        {showActions && (
          <DropdownMenu onOpenChange={setMenuOpen}>
            <DropdownMenuTrigger asChild>
              <button
                type="button"
                data-nodrag
                data-tauri-drag-region="false"
                aria-label="会话操作"
                onClick={(e) => e.stopPropagation()}
                onKeyDown={(e) => e.stopPropagation()}
                className="absolute inset-0 rounded flex items-center justify-center text-neutral-400 invisible group-hover:visible group-focus-within:visible data-[state=open]:visible hover:bg-neutral-300/60 hover:text-neutral-700"
              >
                <MoreHorizontal className="h-3.5 w-3.5" />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent
              side="right"
              align="start"
              sideOffset={4}
              className="min-w-[140px]"
            >
              {renderActionItems(DropdownMenuItem, DropdownMenuSeparator)}
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </div>
    </div>
  );

  // Right-click on the row opens the same menu the "···" button does.
  // Cross-project recents (`showActions === false`) get no menu — fork/delete
  // only address the active workspace's store.
  if (!showActions) return rowEl;
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>{rowEl}</ContextMenuTrigger>
      <ContextMenuContent className="min-w-[140px]">
        {renderActionItems(ContextMenuItem, ContextMenuSeparator)}
      </ContextMenuContent>
    </ContextMenu>
  );
}

/** One project row: click toggles its full conversation list; non-active
 * projects expose a hover "switch to this project" action. */
function ProjectItem({
  project,
  open,
  activeId,
  onToggle,
  onOpenSession,
  onSwitchWorkspace,
  onFork,
  onPin,
  onDelete,
}: {
  project: ProjectOverview;
  open: boolean;
  activeId?: number;
  onToggle: () => void;
  onOpenSession: (root: string, id: number) => void;
  onSwitchWorkspace?: (path: string) => void;
  onFork?: (id: number) => void;
  onPin?: (id: number, pinned: boolean) => void;
  onDelete?: (id: number) => void;
}) {
  return (
    <div className="flex flex-col">
      <div
        data-nodrag
        data-tauri-drag-region="false"
        className={cn(
          open && "sticky top-0 z-10 bg-panel pb-0.5"
        )}
      >
        <TooltipSimple content={project.root} side="right" sideOffset={8}>
          <div
            role="button"
            tabIndex={0}
            data-nodrag
            data-tauri-drag-region="false"
            onClick={onToggle}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onToggle();
              }
            }}
            className="group w-full flex items-center gap-1.5 rounded-md px-2 py-1.5 text-[13px] text-neutral-800 transition-colors text-left cursor-pointer hover:bg-black/[0.04]"
          >
            <span className="text-neutral-400 shrink-0">
              {open ? (
                <ChevronDown className="h-3.5 w-3.5" />
              ) : (
                <ChevronRight className="h-3.5 w-3.5" />
              )}
            </span>
            <span className="text-neutral-500 shrink-0">
              {open ? <FolderOpen className="h-4 w-4" /> : <Folder className="h-4 w-4" />}
            </span>
            <span className="font-medium truncate flex-1 min-w-0">{project.name}</span>
            {!project.current && onSwitchWorkspace && (
              <button
                type="button"
                data-nodrag
                data-tauri-drag-region="false"
                aria-label="切换到此项目"
                onClick={(e) => {
                  e.stopPropagation();
                  onSwitchWorkspace(project.root);
                }}
                onKeyDown={(e) => e.stopPropagation()}
                className="flex-none rounded p-0.5 text-neutral-400 invisible group-hover:visible group-focus-within:visible hover:text-neutral-700 hover:bg-neutral-200/60"
              >
                <FolderOpen className="h-3.5 w-3.5" />
              </button>
            )}
            {project.sessions.length > 0 && (
              <span className="flex-none text-[11px] text-neutral-400 tabular-nums">
                {project.sessions.length}
              </span>
            )}
            {project.current && (
              <span className="w-1.5 h-1.5 rounded-full bg-neutral-800 shrink-0" />
            )}
          </div>
        </TooltipSimple>
      </div>

      {open && (
        <div className="flex flex-col gap-0.5">
          {project.sessions.length === 0 ? (
            <div className="pl-7 h-7 flex items-center text-[12px] text-neutral-400">
              暂无对话记录
            </div>
          ) : (
            project.sessions.map((s) => (
              <SessionItem
                key={s.id}
                row={s}
                isActive={project.current && (activeId ? s.id === activeId : s.active)}
                showActions={project.current}
                onOpen={() => onOpenSession(project.root, s.id)}
                onFork={onFork}
                onPin={onPin}
                onDelete={onDelete}
              />
            ))
          )}
        </div>
      )}
    </div>
  );
}
