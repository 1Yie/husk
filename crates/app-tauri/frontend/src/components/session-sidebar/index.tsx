// Session sidebar — Codex Desktop style sidebar.

import { SquarePen, Search, Clock, Plug, Folder } from "lucide-react";
import type { SessionRow } from "../../types";
import { Orb } from "../agent-orb";
import { Button } from "@/components/ui/button";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { cn } from "@/lib/utils";

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
}: Props) {
  const otherRecents = recentWorkspaces.filter((w) => w.path !== workspaceRoot);

  return (
    <aside className="w-[230px] flex-none flex flex-col bg-neutral-100/70 border-r border-neutral-200 select-none">
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

      {/* Sessions list */}
      <ScrollArea className="flex-1 px-2">
        <div className="flex flex-col gap-0.5 pb-2">
          {sessions.length === 0 ? (
            <div className="px-2.5 py-1.5 text-[13px] text-neutral-400">暂无会话</div>
          ) : (
            sessions.map((s) => (
              <SessionItem
                key={s.id}
                row={s}
                isActive={activeId ? s.id === activeId : s.active}
                onOpen={onOpen}
              />
            ))
          )}
        </div>
      </ScrollArea>

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

      <Separator className="mx-2 w-auto" />

      {/* User / Settings footer */}
      <div className="p-2">
        <Button
          variant="ghost"
          className="w-full justify-start gap-2 h-auto py-2 px-2 hover:bg-neutral-200/60"
        >
          <Avatar className="h-6 w-6">
            <AvatarFallback className="bg-neutral-300 text-neutral-700 text-[11px] font-medium">
              设
            </AvatarFallback>
          </Avatar>
          <div className="flex flex-col items-start leading-tight">
            <span className="text-[13px] font-medium text-neutral-800">设置</span>
            <span className="text-[11px] text-neutral-400">帐户</span>
          </div>
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
}: {
  row: SessionRow;
  isActive: boolean;
  onOpen: (id: number) => void;
}) {
  const displayTitle =
    row.title === "new session" ? "新会话" : (row.title || `对话 ${row.id}`);

  return (
    <Button
      variant={isActive ? "secondary" : "ghost"}
      onClick={() => onOpen(row.id)}
      className={cn(
        "w-full justify-start gap-2 px-2.5 h-8 text-[13px]",
        isActive
          ? "bg-white text-neutral-900 font-medium shadow-sm border border-neutral-200/80 hover:bg-white"
          : "text-neutral-600 hover:bg-neutral-200/50 font-normal"
      )}
    >
      <span className="flex-1 min-w-0 truncate text-left">
        {displayTitle}
      </span>
      {row.running && (
        <Orb
          variant="C3"
          size={14}
          className="text-neutral-800 flex-none"
        />
      )}
    </Button>
  );
}
