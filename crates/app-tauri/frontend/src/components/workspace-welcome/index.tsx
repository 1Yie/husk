// Workspace welcome — the app's initial page (no workspace open yet).
//
// This used to be one icon, one caption and one button in the middle of an
// otherwise empty window. It now carries the primary action, the recent
// workspaces (when there are any), a short list of what the agent can do, and
// the composer's sigils as a preview of the input grammar.
//
// Copy stays plain on purpose: no brand mark, no tagline, no benefit clauses —
// the audience here is the person who is about to open a repo, and every line
// should say something they could act on or verify.
//
// All colors are theme tokens (`bg-card`, `border-hairline`, the inverted
// `neutral-*` ramp), so light and dark need no `dark:` variants here.

import type { ComponentType } from "react";
import {
  Bot,
  Clock,
  FileCode,
  FolderOpen,
  ShieldCheck,
  Terminal,
} from "@keyline-icons/react";
import { Button } from "@/components/ui/button";
import { timeGreeting } from "@/lib/greeting";

export interface RecentWorkspace {
  path: string;
  name: string;
  /** Seconds since epoch — the kernel's `last_opened` stamp. */
  last_opened: number;
}

interface Props {
  recents: RecentWorkspace[];
  onOpenWorkspace: () => void;
  /** Open a workspace straight from the recents grid. */
  onOpenRecent: (path: string) => void;
}

/** What the agent can do once a project is open. Titles are nouns, lines are
 * facts — nothing here sells anything, because the reader is one click from
 * seeing it. */
const CAPABILITIES: { Icon: ComponentType<{ className?: string }>; title: string; desc: string }[] = [
  { Icon: FileCode, title: "代码读写", desc: "读取、搜索和修改项目文件" },
  { Icon: ShieldCheck, title: "沙箱与审批", desc: "命令在沙箱内运行，写操作需批准" },
  { Icon: Bot, title: "多会话", desc: "切换会话时进行中的任务继续运行" },
  { Icon: Terminal, title: "命令与技能", desc: "`/` 命令、`$` 技能、MCP 插件" },
];

/** Relative age of a `last_opened` stamp. Clock skew (a stamp in the future)
 * and pre-epoch zeroes both collapse to 「刚刚」 rather than a negative age. */
function timeAgo(seconds: number): string {
  if (!seconds) return "";
  const elapsed = Date.now() / 1000 - seconds;
  if (elapsed < 60) return "刚刚";
  const minutes = elapsed / 60;
  if (minutes < 60) return `${Math.floor(minutes)} 分钟前`;
  const hours = minutes / 60;
  if (hours < 24) return `${Math.floor(hours)} 小时前`;
  const days = hours / 24;
  if (days < 30) return `${Math.floor(days)} 天前`;
  return new Date(seconds * 1000).toLocaleDateString();
}

export function WorkspaceWelcome({ recents, onOpenWorkspace, onOpenRecent }: Props) {
  const { label, tail, Icon, tone } = timeGreeting();
  return (
    <div className="flex-1 min-h-0 flex flex-col items-center justify-center overflow-y-auto px-6 py-10 select-none">
      <div className="w-full max-w-[720px] flex flex-col items-center gap-8">
        <div className="flex flex-col items-center gap-5 text-center">
          {/* Greeting + tail come from `lib/greeting` — the same line the
              in-session empty state shows, so the two never drift. */}
          <span className="flex items-center gap-2.5 text-[20px] font-semibold tracking-tight text-neutral-900">
            <Icon size={20} className={tone} />
            {label}
            {tail}
          </span>
          <div className="flex flex-col items-center gap-2">
            <Button size="lg" className="rounded-full px-6" onClick={onOpenWorkspace}>
              <FolderOpen className="h-4 w-4" />
              打开工作区…
            </Button>
            {recents.length === 0 && (
              <span className="text-[12px] text-neutral-400">
                或从左侧「项目」打开最近的工作区
              </span>
            )}
          </div>
        </div>

        {recents.length > 0 && (
          <div className="w-full flex flex-col gap-2">
            <span className="flex items-center gap-1.5 px-1 text-[12px] text-neutral-400">
              <Clock className="h-3.5 w-3.5" />
              最近打开
            </span>
            <div className="grid grid-cols-2 gap-2">
              {recents.slice(0, 6).map((r) => (
                <button
                  key={r.path}
                  type="button"
                  title={r.path}
                  onClick={() => onOpenRecent(r.path)}
                  className="group flex items-center gap-3 rounded-[14px] border border-hairline bg-card px-3 py-2.5 text-left cursor-pointer transition-colors hover:bg-hover"
                >
                  <FolderOpen className="h-4 w-4 shrink-0 text-neutral-400 group-hover:text-neutral-600" />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[13px] text-neutral-800">
                      {r.name || r.path}
                    </span>
                    <span className="block truncate text-[11px] text-neutral-400">
                      {r.path}
                    </span>
                  </span>
                  <span className="shrink-0 text-[11px] text-neutral-400">
                    {timeAgo(r.last_opened)}
                  </span>
                </button>
              ))}
            </div>
          </div>
        )}

        <div className="grid w-full grid-cols-2 gap-2.5">
          {CAPABILITIES.map(({ Icon, title, desc }) => (
            <div
              key={title}
              className="rounded-[14px] border border-hairline bg-card px-3.5 py-3"
            >
              <span className="flex items-center gap-2">
                <Icon className="h-3.5 w-3.5 text-neutral-500" />
                <span className="text-[13px] font-medium text-neutral-800">{title}</span>
              </span>
              <span className="mt-1 block text-[12px] leading-relaxed text-neutral-500">
                {desc}
              </span>
            </div>
          ))}
        </div>

        <div className="flex items-center gap-3 text-[12px] text-neutral-400">
          <Sigil k="@" label="引用文件" />
          <Sigil k="/" label="命令" />
          <Sigil k="$" label="技能" />
        </div>
      </div>
    </div>
  );
}

/** A composer sigil rendered as a key cap, matching the textarea's grammar. */
function Sigil({ k, label }: { k: string; label: string }) {
  return (
    <span className="flex items-center gap-1.5">
      <kbd className="flex h-5 min-w-5 items-center justify-center rounded-md border border-hairline bg-card px-1 font-mono text-[11px] text-neutral-600">
        {k}
      </kbd>
      {label}
    </span>
  );
}
