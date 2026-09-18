// Chat Header — top toolbar above the chat stream column.

import {
  MessageSquare,
  MoreHorizontal,
  FolderOpen,
  ChevronDown,
  PanelRight,
  SquarePen,
} from "lucide-react";
import type { SessionRow } from "../../types";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";

interface Props {
  activeSession?: SessionRow;
  workspaceName?: string;
  workspaceRoot?: string;
  onNew?: () => void;
  onPickWorkspace?: () => void;
}

export function ChatHeader({
  activeSession,
  workspaceName,
  workspaceRoot,
  onNew,
  onPickWorkspace,
}: Props) {
  const sessionTitle =
    activeSession?.title === "new session" || !activeSession?.title
      ? "新会话"
      : activeSession.title;
  const wsDisplay = workspaceName || "工作区";

  return (
    <TooltipProvider delayDuration={300}>
      <header className="h-11 flex-none bg-white border-b border-neutral-200 px-3 flex items-center justify-between select-none">
        {/* Left: workspace + session title */}
        <div className="flex items-center gap-2 min-w-0">
          <div className="flex items-center gap-1.5 text-neutral-900 font-semibold text-[13px]">
            <FolderOpen className="h-4 w-4 text-neutral-700 flex-none" />
            <span className="truncate max-w-[220px]" title={workspaceRoot}>
              {wsDisplay}
            </span>
          </div>
          <span className="text-neutral-300 font-light text-[13px]">/</span>
          <span className="text-[13px] text-neutral-600 truncate max-w-[320px]">
            {sessionTitle}
          </span>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="ghost"
                size="icon-sm"
                className="h-6 w-6 text-neutral-400 hover:text-neutral-700"
              >
                <MoreHorizontal className="h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start">
              <DropdownMenuItem>重命名会话</DropdownMenuItem>
              <DropdownMenuItem>导出</DropdownMenuItem>
              <DropdownMenuItem className="text-red-600">删除会话</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>

        {/* Right: location + panel toggles */}
        <div className="flex items-center gap-1.5">
          {onNew && (
            <Button
              variant="outline"
              size="sm"
              onClick={onNew}
              className="h-7 gap-1.5 px-2.5 text-xs font-medium border-neutral-200 text-neutral-700 hover:bg-neutral-100"
            >
              <SquarePen className="h-3.5 w-3.5 text-neutral-500" />
              新建会话
            </Button>
          )}

          <Button
            variant="outline"
            size="sm"
            onClick={onPickWorkspace}
            className="h-7 gap-1.5 px-2.5 text-xs font-medium border-neutral-200 text-neutral-700 hover:bg-neutral-100"
          >
            <FolderOpen className="h-3.5 w-3.5 text-neutral-500" />
            切换目录
          </Button>

          <Separator orientation="vertical" className="h-4 mx-1" />

          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon-sm"
                className="text-neutral-400 hover:text-neutral-700"
              >
                <MoreHorizontal className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="bottom">更多操作</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon-sm"
                className="text-neutral-400 hover:text-neutral-700"
              >
                <PanelRight className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="bottom">侧栏</TooltipContent>
          </Tooltip>
        </div>
      </header>
    </TooltipProvider>
  );
}
