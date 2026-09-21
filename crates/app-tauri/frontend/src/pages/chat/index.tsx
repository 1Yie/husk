// Chat page — the conversation column of the main window: window title
// bar (with the git/usage meter), the scrollable stream, and the composer.
import { useEffect, useRef, useState } from "react";
import { FolderOpen } from "@keyline-icons/react";
import { TitleBar } from "@/components/title-bar";
import { ChatStream } from "@/components/chat-stream";
import { ComposerBar } from "@/components/composer-bar";
import type { SessionView } from "../../hooks/stream-view";
import type { GitInfo } from "../../invoke/agent";

interface ChatPageProps {
  title: string;
  view: SessionView;
  workspaceRoot: string;
  gitInfo?: GitInfo | null;
  contextWindowHint?: number;
  /** Active model's $/1M-token pricing — the header renders the running
   * $ figure against `view.usage` when present. */
  modelCost?: import("../../invoke/agent").ModelItem["cost"];
  /** Session/workspace switch in flight — the stream shows a skeleton. */
  loading?: boolean;
  /** Active session key (`root:id`) — resets the stream's incremental
   * mount window on session AND workspace switches. */
  sessionKey?: string;
  /** Older-history pages exist above the loaded slice. */
  hasMore?: boolean;
  /** Scroll-top handler — fetches + prepends the next older page. */
  onLoadOlder?: () => Promise<boolean>;
  /** Opens the raw-JSON history viewer. */
  onShowRaw?: () => void;
  /** No workspace open — the empty pane's "打开工作区" button. */
  onOpenWorkspace?: () => void;
  /** Boot not resolved — keep the skeleton instead of the empty pane. */
  workspaceReady?: boolean;
}

export function ChatPage({ title, view, workspaceRoot, gitInfo, contextWindowHint, modelCost, loading, sessionKey, hasMore, onLoadOlder, onShowRaw, onOpenWorkspace, workspaceReady = true }: ChatPageProps) {
  // The composer floats over the stream bottom — measure its real height
  // (card + pb-6 gap + the taller approval/todo banner variants) and feed
  // it to the stream as bottom padding, so the last message can always
  // scroll above the card instead of sliding under it.
  const [composerH, setComposerH] = useState(160);
  const composerRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = composerRef.current;
    if (!el) return;
    const updateHeight = () => {
      const h = el.offsetHeight || el.getBoundingClientRect().height;
      if (h > 0) {
        setComposerH(h);
      }
    };
    updateHeight();
    const ro = new ResizeObserver(updateHeight);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  return (
    <>
      <TitleBar title={title} view={view} gitInfo={gitInfo} contextWindowHint={contextWindowHint} modelCost={modelCost} onShowRaw={onShowRaw} noWorkspace={!workspaceRoot} />
      {!workspaceRoot ? (
        workspaceReady ? (
          <NoWorkspacePane onOpenWorkspace={onOpenWorkspace} />
        ) : (
          <div className="flex-1 min-h-0" />
        )
      ) : (
        <>
      <ChatStream
        // Remount per session — scroll position, pin state, landing
        // flag and the windowing shell state are all per-session.
        key={sessionKey}
        view={view}
        bottomPad={composerH + 24}
        composerH={composerH}
        loading={loading}
        sessionKey={sessionKey}
        hasMore={hasMore}
        onLoadOlder={onLoadOlder}
      />

      {/* Bottom gradient mask: subtle, soft dissolve behind the floating composer */}
      <div
        style={{ height: composerH + 16 }}
        className="absolute inset-x-0 bottom-0 pointer-events-none z-10 bg-gradient-to-t from-[color-mix(in_srgb,var(--husk-white)_75%,transparent)] via-[color-mix(in_srgb,var(--husk-white)_35%,transparent)] to-transparent dark:from-[#16161a]/75 dark:via-[#16161a]/35 dark:to-transparent"
        aria-hidden="true"
      />

      <div ref={composerRef} className="absolute inset-x-0 bottom-0 pointer-events-none z-20">
        <ComposerBar view={view} workspaceRoot={workspaceRoot} sessionKey={sessionKey} />
      </div>
        </>
      )}
    </>
  );
}

function NoWorkspacePane({ onOpenWorkspace }: { onOpenWorkspace?: () => void }) {
  return (
    <div className="flex-1 min-h-0 flex flex-col items-center justify-center gap-3 select-none pb-16">
      <FolderOpen className="h-8 w-8 text-neutral-300" />
      <span className="text-[15px] font-medium text-neutral-700">
        没有打开的工作区
      </span>
      <span className="text-[13px] text-neutral-400">
        打开一个项目文件夹开始，或从左侧「项目」选择最近的工作区
      </span>
      {onOpenWorkspace && (
        <button
          type="button"
          onClick={onOpenWorkspace}
          className="mt-1 h-8 px-4 rounded-lg border border-neutral-200 text-[13px] text-neutral-700 hover:bg-neutral-50 transition-colors cursor-pointer"
        >
          打开工作区…
        </button>
      )}
    </div>
  );
}
