import iconUrl from "@/assets/husk-icon.png";
import {
  Plus,
  ChevronRight,
  ChevronDown,
  Folder,
  FolderOpen,
  FolderPlus,
  Settings,
  Bell,
  MoreHorizontal,
  GitFork,
  Bin,
  Bookmark,
  X,
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
import {
  useNotifications,
  useUnreadCount,
  markRead,
  markAllRead,
  type AppNotification,
} from "../../lib/notifications";

/** Cross-project recent list cap — everything beyond it stays reachable by
 * opening the owning project in 项目. */
const RECENT_LIMIT = 6;

interface Props {
  /** Project tree — active workspace first, then recent ones, each with its
   * full conversation list (`SessionManager::projects_overview`). */
  projects: ProjectOverview[];
  activeId?: number;
  version?: string;
  hasWorkspace?: boolean;
  onNew: () => void;
  /** Open a conversation anywhere in the tree — `root` lets the shell switch
   * workspaces when the row belongs to another project. */
  onOpenSession: (root: string, id: number) => void;
  onPickWorkspace?: () => void;
  onSwitchWorkspace?: (path: string) => void;
  onRemoveWorkspace?: (project: ProjectOverview) => void;
  onOpenSettings?: () => void;
  onFork?: (id: number) => void;
  onPin?: (id: number, pinned: boolean) => void;
  onDelete?: (id: number) => void;
}

export function SessionSidebar({
  projects,
  activeId,
  version: propVersion,
  hasWorkspace = true,
  onNew,
  onOpenSession,
  onPickWorkspace,
  onSwitchWorkspace,
  onRemoveWorkspace,
  onOpenSettings,
  onFork,
  onPin,
  onDelete,
}: Props) {
  const notifications = useNotifications();
  const unreadCount = useUnreadCount();
  // Resolve a notification's session to its sidebar title — the store only
  // carries root+id; names come from the same project tree the rows use.
  const titleFor = (n: AppNotification): string => {
    if (n.root === undefined || n.session === undefined) return n.title;
    for (const p of projects) {
      if (p.root !== n.root) continue;
      const row = p.sessions.find((s) => s.id === n.session);
      if (row) return `${n.title} — ${row.title}`;
    }
    return `${n.title} — 会话 #${n.session}`;
  };
  const openNotification = (n: AppNotification) => {
    markRead(n.id);
    if (n.root !== undefined && n.session !== undefined) {
      onOpenSession(n.root, n.session);
    }
  };
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

          <img src={iconUrl} alt="" draggable={false} className="h-4 w-4 shrink-0" />
          <span className="text-[12px] font-semibold text-neutral-700 tracking-tight">
            Husk
          </span>
        </div>

        <div className="flex-1 min-h-0 flex flex-col">
          <div className="flex-none flex items-center justify-between px-4 pt-3 pb-1">
            <span className="text-[13px] font-medium text-neutral-600">
              会话
            </span>
          </div>

          {/* Recent sessions — newest first across every project, capped so
              the full archive stays inside 项目. */}
          <div className="flex-none flex flex-col gap-0.5 px-2 pb-1">
            {recents.length === 0 ? (
              <div className="pl-7 h-8 flex items-center shrink-0 flex-none text-[13px] text-neutral-500">
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

          {/* Section: 项目 — open a project to read its full conversation list.
              Right side: "+" starts a session in the current workspace, the
              folder icon picks a different workspace directory. */}
          <div className="flex-none flex items-center justify-between px-4 pt-2 pb-1">
            <span className="text-[13px] font-medium text-neutral-600">
              项目
            </span>
            <div className="flex items-center gap-0.5">
              {hasWorkspace && (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <button
                      type="button"
                      data-nodrag
                      data-tauri-drag-region="false"
                      onClick={onNew}
                      aria-label="新建会话"
                      className="h-5 w-5 rounded flex items-center justify-center text-neutral-500 hover:text-neutral-700 hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] transition-colors"
                    >
                      <Plus className="h-3.5 w-3.5" />
                    </button>
                  </TooltipTrigger>
                  <TooltipContent side="right" className="text-xs">
                    新建会话
                  </TooltipContent>
                </Tooltip>
              )}
              {onPickWorkspace && (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <button
                      type="button"
                      data-nodrag
                      data-tauri-drag-region="false"
                      onClick={onPickWorkspace}
                      aria-label="打开其他工作区"
                      className="h-5 w-5 rounded flex items-center justify-center text-neutral-500 hover:text-neutral-700 hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] transition-colors"
                    >
                      <FolderPlus className="h-3.5 w-3.5" />
                    </button>
                  </TooltipTrigger>
                  <TooltipContent side="right" className="text-xs">
                    打开其他工作区
                  </TooltipContent>
                </Tooltip>
              )}
            </div>
          </div>

          {/* Project list: the content area's only scroller. */}
          <div data-tauri-drag-region="false" className="flex-1 min-h-0 overflow-y-auto transform-gpu flex flex-col gap-0.5 px-2 pb-2">
            {projects.length === 0 ? (
              <div className="pl-3 h-8 flex items-center text-[13px] text-neutral-500">
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
                  onRemoveWorkspace={onRemoveWorkspace}
                  onFork={onFork}
                  onPin={onPin}
                  onDelete={onDelete}
                />
               ))
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
                  className="text-neutral-500 hover:text-neutral-800 transition-colors p-1 rounded hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)]"
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
                      className="relative text-neutral-500 hover:text-neutral-800 transition-colors p-1 rounded hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)]"
                    >
                      <Bell className="h-4 w-4" />
                      {unreadCount > 0 && (
                        <span className="absolute -top-0.5 -right-0.5 min-w-3.5 h-3.5 px-0.5 rounded-full bg-red-500 text-zinc-50 text-[9px] font-semibold leading-[14px] text-center select-none">
                          {unreadCount > 99 ? "99+" : unreadCount}
                        </span>
                      )}
                    </button>
                  </PopoverTrigger>
                </TooltipTrigger>
                <TooltipContent side="top" className="text-xs">
                  通知
                </TooltipContent>
              </Tooltip>
              <PopoverContent side="top" align="start" className="w-64 p-0 text-xs overflow-hidden">
                <div className="flex items-center justify-between px-3 pt-2.5 pb-2">
                  <div className="font-semibold text-neutral-800">系统通知</div>
                  {unreadCount > 0 && (
                    <button
                      type="button"
                      onClick={() => markAllRead()}
                      className="text-[11px] text-neutral-500 hover:text-neutral-700 transition-colors cursor-pointer"
                    >
                      全部已读
                    </button>
                  )}
                </div>
                {notifications.length === 0 ? (
                  <div className="px-3 pb-3 text-neutral-500">暂无新通知。</div>
                ) : (
                  <div data-tauri-drag-region="false" className="max-h-64 overflow-y-auto border-t border-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)]">
                    {notifications.map((n) => (
                      <button
                        key={n.id}
                        type="button"
                        onClick={() => openNotification(n)}
                        className="w-full flex items-start gap-2 px-3 py-2 text-left hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)] transition-colors cursor-pointer"
                      >
                        {!n.read && (
                          <span className="mt-1.5 h-1.5 w-1.5 flex-none rounded-full bg-blue-500" />
                        )}
                        <span className={cn("min-w-0 flex-1", n.read && "pl-3.5")}>
                          <span className={cn("block truncate", n.read ? "text-neutral-500" : "text-neutral-800 font-medium")}>
                            {titleFor(n)}
                          </span>
                          {n.body && (
                            <span className="block truncate text-neutral-500 mt-0.5">
                              {n.body}
                            </span>
                          )}
                          <span className="block text-neutral-500 mt-0.5 text-[10.5px]">
                            {new Date(n.ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
                          </span>
                        </span>
                      </button>
                    ))}
                  </div>
                )}
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
          ? "bg-[color-mix(in_srgb,var(--husk-black)_6%,transparent)] text-neutral-900"
          : "text-neutral-600 hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)] hover:text-neutral-900 font-normal",
      )}
    >
      <span className="flex-1 min-w-0 truncate text-left">{displayTitle}</span>
      {row.pinned && (
        <Bookmark className="h-3 w-3 flex-none fill-current text-amber-500 dark:text-amber-400" />
      )}
      {hint && (
        <span className="flex-none max-w-[64px] truncate text-[11px] text-neutral-500">
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
                className="absolute inset-0 rounded flex items-center justify-center text-neutral-500 invisible group-hover:visible group-focus-within:visible data-[state=open]:visible hover:bg-[color-mix(in_srgb,var(--husk-n300)_60%,transparent)] hover:text-neutral-700"
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
  onRemoveWorkspace,
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
  onRemoveWorkspace?: (project: ProjectOverview) => void;
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
            className="group w-full flex items-center gap-1.5 rounded-md px-2 py-1.5 text-[13px] text-neutral-800 transition-colors text-left cursor-pointer hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)]"
          >
            <span className="text-neutral-500 shrink-0">
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
                className="flex-none rounded p-0.5 text-neutral-500 invisible group-hover:visible group-focus-within:visible hover:text-neutral-700 hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)]"
              >
                <FolderOpen className="h-3.5 w-3.5" />
              </button>
            )}
            {onRemoveWorkspace && (
              <button
                type="button"
                data-nodrag
                data-tauri-drag-region="false"
                aria-label="移除项目"
                onClick={(e) => {
                  e.stopPropagation();
                  onRemoveWorkspace(project);
                }}
                onKeyDown={(e) => e.stopPropagation()}
                className="flex-none rounded p-0.5 text-neutral-500 invisible group-hover:visible group-focus-within:visible hover:text-neutral-700 hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)]"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            )}
            {project.sessions.length > 0 && (
              <span className="flex-none text-[11px] text-neutral-500 tabular-nums">
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
            <div className="pl-7 h-7 flex items-center text-[12px] text-neutral-500">
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
