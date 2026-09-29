import iconUrl from "@/assets/husk-icon.png";
import {
  Plus,
  Bot,
  ChevronRight,
  ChevronDown,
  FileText,
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
import { Download, ExternalLink } from "lucide-react";
import { toast } from "sonner";
import type { ProjectOverview, SessionRow } from "@/types";
import { Orb } from "@/features/chat/components/agent-orb/index";
import { ExtIcon } from "@/features/chat/components/artifacts-panel";
import {
  openPath,
  recentArtifacts,
  revealPath,
  saveArtifact,
  type RecentArtifact,
} from "@/lib/agent-ipc/index";
import { useActiveView } from "@/stores/agent-store";
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
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { isMac } from "@/lib/platform";
import { cn } from "@/lib/utils";
import {
  useNotifications,
  useUnreadCount,
  markRead,
  markAllRead,
  type AppNotification,
} from "@/lib/notifications";

/** Cross-project recent list cap — everything beyond it stays reachable by
 * opening the owning project in 项目. */
const RECENT_LIMIT = 6;

/** 产物 categories — the file tree's fixed buckets. `rep` is the ext used
 * for the group's icon; `asset` also matches anything under an
 * `assets/` directory (the mode prompt puts media there regardless of
 * ext). */
const ART_GROUPS: { key: string; label: string; rep: string }[] = [
  { key: "pdf", label: "PDF", rep: "pdf" },
  { key: "doc", label: "DOC", rep: "docx" },
  { key: "ppt", label: "PPT", rep: "pptx" },
  { key: "xls", label: "XLS", rep: "xlsx" },
  { key: "asset", label: "素材", rep: "png" },
  { key: "other", label: "其他", rep: "" },
];
function artCategory(f: RecentArtifact): string {
  if (/(^|\/)assets\//.test(f.path)) return "asset";
  switch ((f.name.split(".").pop() ?? "").toLowerCase()) {
    case "pdf":
      return "pdf";
    case "docx":
    case "doc":
    case "md":
    case "txt":
    case "rtf":
      return "doc";
    case "pptx":
    case "ppt":
      return "ppt";
    case "xlsx":
    case "xls":
    case "csv":
      return "xls";
    case "png":
    case "jpg":
    case "jpeg":
    case "gif":
    case "webp":
    case "svg":
    case "bmp":
    case "mp4":
    case "mov":
    case "webm":
    case "mp3":
    case "wav":
    case "flac":
    case "ttf":
    case "otf":
    case "woff":
    case "woff2":
      return "asset";
    default:
      return "other";
  }
}

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
  // The office pseudo-workspace arrives in the same project list flagged
  // `office`. It's never a project row — it drives the header tab (工作 ↔
  // 编程 switch) and, while office mode is active, supplies the flat
  // session archive rendered where the project tree normally sits.
  const officeProject = projects.find((p) => p.office);
  const projectList = projects.filter((p) => !p.office);
  const officeMode = officeProject?.current === true;
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
  // Global recents: every *project's* conversations merged — pinned first
  // (each tier newest-first), so the top of the sidebar always answers
  // "what was I just doing" while pinned sessions stay reachable. Office
  // sessions stay out of it — modes are separate: the 会话 list is
  // build-mode content, and office sessions only ever show on the 工作 tab.
  const recents = useMemo(() => {
    const all = projectList.flatMap((p) =>
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
  }, [projectList]);

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

  // 产物 list — office mode's counterpart of the project tree: files the
  // sessions produced, grouped by day (今天/昨天/date) like projects group
  // conversations. Refetches whenever the active view registers a new
  // artifact mid-turn, when a session's stamp bumps, or on tab entry.
  const view = useActiveView();
  const viewArtifactCount = officeMode ? (view?.artifacts?.length ?? 0) : 0;
  const sessionsStamp = officeMode
    ? (officeProject?.sessions.map((s) => `${s.id}:${s.updated_at}`).join("|") ?? "")
    : "";
  const [officeFiles, setOfficeFiles] = useState<RecentArtifact[] | null>(null);
  useEffect(() => {
    if (!officeMode) {
      setOfficeFiles(null);
      return;
    }
    let alive = true;
    void recentArtifacts()
      .then((r) => alive && setOfficeFiles(r))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [officeMode, viewArtifactCount, sessionsStamp]);

  interface ArtGroup {
    key: string;
    label: string;
    /** Representative ext — drives the group's ExtIcon. */
    rep: string;
    files: RecentArtifact[];
  }
  interface ArtDayGroup {
    key: string;
    label: string;
    types: ArtGroup[];
  }
  const artifactGroups = useMemo<ArtDayGroup[]>(() => {
    // Top level: calendar days (今天/昨天/M月D日) — the time classification.
    const days: { key: string; label: string; files: RecentArtifact[] }[] = [];
    const startOfDay = (t: number) => {
      const d = new Date(t);
      d.setHours(0, 0, 0, 0);
      return d.getTime();
    };
    const today = startOfDay(Date.now());
    for (const f of officeFiles ?? []) {
      const day = startOfDay(f.mtime * 1000);
      const key = String(day);
      let g = days.find((x) => x.key === key);
      if (!g) {
        const diff = Math.round((today - day) / 86400000);
        const d = new Date(f.mtime * 1000);
        g = {
          key,
          label:
            diff <= 0
              ? "今天"
              : diff === 1
                ? "昨天"
                : d.getFullYear() === new Date().getFullYear()
                  ? `${d.getMonth() + 1}月${d.getDate()}日`
                  : `${d.getFullYear()}年${d.getMonth() + 1}月${d.getDate()}日`,
          files: [],
        };
        days.push(g);
      }
      g.files.push(f);
    }
    // Second level: inside each day, split into the fixed type buckets
    // (PDF / DOC / PPT / 素材 / 其他) — files keep mtime-desc order.
    return days.map((g) => ({
      key: g.key,
      label: g.label,
      types: ART_GROUPS.map((t) => ({
        ...t,
        files: g.files.filter((f) => artCategory(f) === t.key),
      })).filter((t) => t.files.length > 0),
    }));
  }, [officeFiles]);
  // Only the newest day starts expanded — mirrors the current project
  // opening by default while older days stay folded.
  const [openArt, setOpenArt] = useState<Set<string> | null>(null);
  useEffect(() => {
    if (openArt === null && artifactGroups.length > 0) {
      setOpenArt(new Set([artifactGroups[0].key]));
    }
  }, [artifactGroups, openArt]);
  const toggleArtGroup = (key: string) =>
    setOpenArt((prev) => {
      const next = new Set(prev ?? []);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });

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
          {isMac && <div className="w-[60px] shrink-0" />}

          <img src={iconUrl} alt="" draggable={false} className="h-4 w-4 shrink-0" />
          <span className="text-[12px] font-semibold text-neutral-700 tracking-tight">
            Husk
          </span>

          {/* Mode tabs — the 编程 ↔ 工作 switch lives in the sidebar header,
              not the composer. 工作 targets the built-in office workspace
              (~/.local/share/husk/office); 编程 returns to the most recent
              real project (the folder picker covers the no-project case). */}
          {onSwitchWorkspace && (
            <Tabs
              value={officeMode ? "office" : "build"}
              onValueChange={(v) => {
                if (v === "office") {
                  if (officeProject) onSwitchWorkspace(officeProject.root);
                } else {
                  const target = projectList[0]?.root;
                  if (target) onSwitchWorkspace(target);
                  else onPickWorkspace?.();
                }
              }}
              className="ml-auto"
            >
              <TabsList
                data-nodrag
                data-tauri-drag-region="false"
                className="relative h-auto gap-0 rounded-md bg-[color-mix(in_srgb,var(--husk-black)_5%,transparent)] p-0.5"
              >
                {/* Sliding active pill — triggers are fixed-width so the
                    translate is a constant. This theme overrides
                    rounded-md to 10px — the pill's 8px (= 10 − the 2px
                    p-0.5 ring) keeps the nested radii concentric. */}
                <span
                  aria-hidden
                  className={cn(
                    "absolute inset-y-0.5 left-0.5 w-12 rounded-[8px] bg-white shadow-[0_1px_2px_rgba(0,0,0,0.08)] transition-transform duration-200 ease-out will-change-transform transform-gpu",
                    officeMode && "translate-x-12",
                  )}
                />
                <TabsTrigger
                  value="build"
                  data-nodrag
                  aria-label="切换到编程模式"
                  className="relative h-[22px] w-12 gap-1 rounded px-0 py-0 text-[11px] font-normal text-neutral-500 transition-none hover:text-neutral-700 data-[state=active]:bg-transparent data-[state=active]:text-neutral-800 data-[state=active]:shadow-none"
                >
                  <Bot className="h-3 w-3" />
                  编程
                </TabsTrigger>
                <TabsTrigger
                  value="office"
                  data-nodrag
                  aria-label="切换到工作模式"
                  disabled={!officeProject}
                  className="relative h-[22px] w-12 gap-1 rounded px-0 py-0 text-[11px] font-normal text-neutral-500 transition-none hover:text-neutral-700 disabled:opacity-100 data-[state=active]:bg-transparent data-[state=active]:text-neutral-800 data-[state=active]:shadow-none"
                >
                  <FileText className="h-3 w-3" />
                  工作
                </TabsTrigger>
              </TabsList>
            </Tabs>
          )}
        </div>

        <div className="flex-1 min-h-0 flex flex-col">
          {/* The merged-recents 会话 block is build-mode only — office mode's
              scroll area IS the office archive, so a cross-workspace top-6
              would just duplicate it (and leak project rows into a mode
              that hides them). */}
          {!officeMode && (
            <>
              <div className="flex-none flex items-center justify-between px-4 pt-3 pb-1">
                <span className="text-[13px] font-medium text-neutral-600">
                  会话
                </span>
              </div>
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
            </>
          )}

          {/* Section header —「项目」in build mode,「会话」in office mode
              (the scroll area below it is then the office session archive;
              no project tree shows there). "+" starts a session in the
              active workspace either way; the folder picks a project dir
              and so only exists in build mode. */}
          <div className={cn(
            "flex-none flex items-center justify-between px-4 pb-1",
            officeMode ? "pt-3" : "pt-2",
          )}>
            <span className="text-[13px] font-medium text-neutral-600">
              {officeMode ? "会话" : "项目"}
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
              {onPickWorkspace && !officeMode && (
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

          {officeMode ? (
            /* 工作: sessions shrink-wrap to a capped strip; the 产物 file
               tree owns the remaining scroller — the same "header then
               contents" split the 项目 tree gives build mode. */
            <>
              <div data-tauri-drag-region="false" className="flex-none max-h-[45%] overflow-y-auto">
                <div className="flex flex-col gap-0.5 px-2 pb-1">
                  {officeProject && officeProject.sessions.length === 0 ? (
                    <div className="pl-7 h-8 flex items-center text-[13px] text-neutral-500">
                      暂无会话
                    </div>
                  ) : (
                    officeProject?.sessions.map((s) => (
                      <SessionItem
                        key={s.id}
                        row={s}
                        isActive={activeId ? s.id === activeId : s.active}
                        showActions
                        onOpen={() => onOpenSession(officeProject.root, s.id)}
                        onFork={onFork}
                        onPin={onPin}
                        onDelete={onDelete}
                      />
                    ))
                  )}
                </div>
              </div>
              <div className="flex-none flex items-center px-4 pt-2 pb-1">
                <span className="text-[13px] font-medium text-neutral-600">产物</span>
              </div>
              {/* Same scroller rules as the project list — `false` outer,
                  `deep` inner so the empty tail still drags the window. */}
              <div data-tauri-drag-region="false" className="flex-1 min-h-0 overflow-y-auto transform-gpu">
                <div data-tauri-drag-region="deep" className="min-h-full flex flex-col gap-0.5 px-2 pb-2">
                  {artifactGroups.length === 0 ? (
                    <div className="pl-7 h-8 flex items-center text-[13px] text-neutral-500">
                      暂无产物
                    </div>
                  ) : (
                    artifactGroups.map((g) => (
                      <OfficeFileGroup
                        key={g.key}
                        label={g.label}
                        types={g.types}
                        open={openArt?.has(g.key) ?? false}
                        onToggle={() => toggleArtGroup(g.key)}
                      />
                    ))
                  )}
                </div>
              </div>
            </>
          ) : (
            /* Project list: the content area's only scroller. It must stay
                `false` — the scrollbar's mousedown targets this element, and a
                drag region here makes scrollbar drags move the window. The
                inner wrapper re-enables `deep` so row gaps and the empty tail
                still drag the window (rows opt out themselves). */
            <div data-tauri-drag-region="false" className="flex-1 min-h-0 overflow-y-auto transform-gpu">
              <div data-tauri-drag-region="deep" className="min-h-full flex flex-col gap-0.5 px-2 pb-2">
                {projectList.length === 0 ? (
                  // Same inset as 暂无会话 — the two empty states are the same kind
                  // of thing, so they read at the same level.
                  <div className="pl-7 h-8 flex items-center text-[13px] text-neutral-500">
                    暂无项目
                  </div>
                ) : (
                  projectList.map((p) => (
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
          )}
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
        "group relative flex-none shrink-0 flex w-full items-center gap-1.5 rounded-md pl-7 pr-2 h-8 min-h-8 text-[13px] cursor-pointer select-none transition-colors",
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
      {/* Trailing: the running orb keeps its slot; the more menu floats
          over the row's right edge on hover/focus — a non-running row
          reserves nothing (the old fixed w-5 cell read as a dead gap). */}
      {row.running && (
        <span className="relative flex-none h-5 w-5">
          {!menuOpen && (
            <Orb
              variant="C3"
              size={14}
              className="absolute inset-0 flex items-center justify-center text-neutral-800 group-hover:invisible group-focus-visible:invisible"
            />
          )}
        </span>
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
              // right-2 lands it exactly on the orb's rest position
              // (pr-2 + w-5); the button also backs itself with a panel
              // blur so a long title doesn't bleed through.
              className="absolute right-2 top-1/2 h-5 w-5 -translate-y-1/2 rounded flex items-center justify-center text-neutral-500 invisible group-hover:visible group-focus-visible:visible data-[state=open]:visible bg-[color-mix(in_srgb,var(--husk-panel)_85%,transparent)] backdrop-blur-sm hover:bg-[color-mix(in_srgb,var(--husk-n300)_60%,transparent)] hover:text-neutral-700"
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

/** One day group in the office 产物 tree — mirrors `ProjectItem`: a
 * collapsible header (chevron + folder + label + count) whose children are
 * type sub-groups (PDF/DOC/PPT/素材), each collapsible down to file rows. */
function OfficeFileGroup({
  label,
  types,
  open,
  onToggle,
}: {
  label: string;
  types: { key: string; label: string; rep: string; files: RecentArtifact[] }[];
  open: boolean;
  onToggle: () => void;
}) {
  const count = types.reduce((n, t) => n + t.files.length, 0);
  // Type sub-groups start COLLAPSED — expanding a day shouldn't flood the
  // sidebar with every file at once; the user opens the bucket they need.
  const [closedTypes, setClosedTypes] = useState<Set<string>>(
    () => new Set(types.map((t) => t.key)),
  );
  const toggleType = (key: string) =>
    setClosedTypes((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  return (
    <div className="flex flex-col">
      <div
        data-nodrag
        data-tauri-drag-region="false"
        className={cn(open && "sticky top-0 z-10 bg-panel pb-0.5")}
      >
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
          <span className="font-medium truncate flex-1 min-w-0">{label}</span>
          <span className="flex-none text-[11px] text-neutral-500 tabular-nums">
            {count}
          </span>
        </div>
      </div>
      {open && (
        <div className="flex flex-col gap-0.5">
          {types.map((t) => {
            const tOpen = !closedTypes.has(t.key);
            return (
              <div key={t.key} className="flex flex-col">
                <div
                  role="button"
                  tabIndex={0}
                  data-nodrag
                  data-tauri-drag-region="false"
                  onClick={() => toggleType(t.key)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      toggleType(t.key);
                    }
                  }}
                  className="group relative flex w-full items-center gap-1.5 rounded-md pl-4 pr-2 h-8 min-h-8 text-[13px] cursor-pointer select-none transition-colors text-neutral-600 hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)] hover:text-neutral-900"
                >
                  <span className="text-neutral-400 shrink-0">
                    {tOpen ? (
                      <ChevronDown className="h-3 w-3" />
                    ) : (
                      <ChevronRight className="h-3 w-3" />
                    )}
                  </span>
                  <ExtIcon ext={t.rep} />
                  <span className="flex-1 min-w-0 truncate text-[12px]">{t.label}</span>
                  <span className="flex-none text-[11px] text-neutral-500 tabular-nums">
                    {t.files.length}
                  </span>
                </div>
                {tOpen && (
                  <div className="flex flex-col gap-0.5">
                    {t.files.map((f) => (
                      <OfficeFileRow key={f.path} file={f} />
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

/** One produced file row under an 产物 date group — click opens it in the
 * OS; the hover chip carries 位置/另存为 like the artifacts dock. */
function OfficeFileRow({ file }: { file: RecentArtifact }) {
  const dir = file.path.includes("/")
    ? file.path.slice(0, file.path.lastIndexOf("/"))
    : "";
  const open = () =>
    void openPath(file.path).catch((err) =>
      toast.error(`打开失败：${err instanceof Error ? err.message : err}`),
    );
  return (
    <div
      role="button"
      tabIndex={0}
      data-nodrag
      data-tauri-drag-region="false"
      onClick={open}
      onKeyDown={(e) => {
        if (e.key === "Enter") open();
      }}
      title={file.path}
      className="group relative flex w-full items-center gap-1.5 rounded-md pl-9 pr-2 h-8 min-h-8 text-[13px] cursor-pointer select-none transition-colors text-neutral-600 hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)] hover:text-neutral-900"
    >
      <ExtIcon ext={file.name.split(".").pop() ?? ""} />
      <span className="flex-1 min-w-0 truncate">{file.name}</span>
      {dir && (
        <span className="flex-none max-w-[72px] truncate text-[11px] text-neutral-500">
          {dir}
        </span>
      )}
      {/* Floating hover actions — same pattern as the dock: absolute over
          the row's right edge, no reserved space. */}
      <span className="absolute right-1 top-1/2 hidden -translate-y-1/2 items-center gap-0.5 rounded-md bg-[color-mix(in_srgb,var(--husk-panel)_85%,transparent)] p-0.5 backdrop-blur-sm group-hover:flex group-focus-visible:flex">
        <button
          type="button"
          aria-label="打开"
          onClick={(e) => {
            e.stopPropagation();
            open();
          }}
          className="flex h-5 w-5 items-center justify-center rounded text-neutral-500 hover:bg-[color-mix(in_srgb,var(--husk-n300)_60%,transparent)] hover:text-neutral-700"
        >
          <ExternalLink className="h-3 w-3" />
        </button>
        <button
          type="button"
          aria-label="另存为"
          onClick={(e) => {
            e.stopPropagation();
            void saveArtifact(file.path).catch((err) =>
              toast.error(`另存为失败：${err instanceof Error ? err.message : err}`),
            );
          }}
          className="flex h-5 w-5 items-center justify-center rounded text-neutral-500 hover:bg-[color-mix(in_srgb,var(--husk-n300)_60%,transparent)] hover:text-neutral-700"
        >
          <Download className="h-3 w-3" />
        </button>
        <button
          type="button"
          aria-label="在文件夹中显示"
          onClick={(e) => {
            e.stopPropagation();
            void revealPath(file.path).catch((err) =>
              toast.error(`打开文件夹失败：${err instanceof Error ? err.message : err}`),
            );
          }}
          className="flex h-5 w-5 items-center justify-center rounded text-neutral-500 hover:bg-[color-mix(in_srgb,var(--husk-n300)_60%,transparent)] hover:text-neutral-700"
        >
          <FolderOpen className="h-3 w-3" />
        </button>
      </span>
    </div>
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
            className="group relative w-full flex items-center gap-1.5 rounded-md px-2 py-1.5 text-[13px] text-neutral-800 transition-colors text-left cursor-pointer hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)]"
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
            {/* Hover actions float over the count badge / current dot —
                `invisible`-in-flow used to reserve ~40px of dead space on
                every row. */}
            <span className="absolute right-1 top-1/2 hidden -translate-y-1/2 items-center gap-0.5 rounded-md bg-[color-mix(in_srgb,var(--husk-panel)_85%,transparent)] p-0.5 backdrop-blur-sm group-hover:flex group-focus-visible:flex">
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
                  className="flex-none rounded p-0.5 text-neutral-500 hover:text-neutral-700 hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)]"
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
                  className="flex-none rounded p-0.5 text-neutral-500 hover:text-neutral-700 hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)]"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              )}
            </span>
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
