// Chat page — the conversation column of the main window: window title
// bar (with the git/usage meter), the scrollable stream, and the composer.
import { useCallback, useRef, useState } from "react";
import { FolderOpen } from "@keyline-icons/react";
import { TitleBar } from "@/components/title-bar";
import { ChatStream } from "@/components/chat-stream";
import { ComposerBar } from "@/components/composer-bar";
import { WorkspaceWelcome, type RecentWorkspace } from "@/components/workspace-welcome";
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
  /** No workspace open — the welcome page's "打开工作区" button. */
  onOpenWorkspace?: () => void;
  /** Recent workspaces for the welcome page's one-click reopen grid. */
  recents?: RecentWorkspace[];
  /** Reopen a recent workspace directly (parked projects keep their state). */
  onOpenRecent?: (path: string) => void;
  /** Boot not resolved — keep the skeleton instead of the empty pane. */
  workspaceReady?: boolean;
}

export function ChatPage({ title, view, workspaceRoot, gitInfo, contextWindowHint, modelCost, loading, sessionKey, hasMore, onLoadOlder, onShowRaw, onOpenWorkspace, recents, onOpenRecent, workspaceReady = true }: ChatPageProps) {
  // The composer floats over the stream bottom — measure its real height
  // (card + pb-6 gap + the taller approval/todo banner variants) and feed
  // it to the stream as bottom padding, so the last message can always
  // scroll above the card instead of sliding under it.
  //
  // The observer attaches from a CALLBACK ref, not `useRef` + a one-shot
  // effect. The measured node does not exist on the first render: boot opens
  // on the welcome page (`workspaceRoot === ""`) and the composer branch only
  // mounts once a workspace resolves. An effect grabbing `composerRef.current`
  // ran while that ref was still null, bailed on the `!el` guard, and — with
  // `[]` deps — never ran again, so the ResizeObserver stayed unattached for
  // the whole session and `composerH` stayed pinned at its 160px initial
  // value. Whenever a panel opened (approval / ask_question / task list /
  // queued messages) the composer grew past that frozen reservation and the
  // last message slid under it. Keying the observer to the node itself keeps
  // it honest across mounts.
  const [composerH, setComposerH] = useState(160);
  const composerRoRef = useRef<ResizeObserver | null>(null);
  const measureComposer = useCallback((el: HTMLDivElement | null) => {
    composerRoRef.current?.disconnect();
    composerRoRef.current = null;
    if (!el) return;
    // Re-fires on every size change — the composer's height is live (textarea
    // autogrow, panels opening/closing, todo list collapse), and the stream
    // re-pins off the padding this feeds.
    const updateHeight = () => {
      const h = el.offsetHeight || el.getBoundingClientRect().height;
      if (h > 0) {
        setComposerH(h);
      }
    };
    updateHeight();
    const ro = new ResizeObserver(updateHeight);
    ro.observe(el);
    composerRoRef.current = ro;
  }, []);

  return (
    <>
      <TitleBar title={title} view={view} gitInfo={gitInfo} contextWindowHint={contextWindowHint} modelCost={modelCost} onShowRaw={onShowRaw} noWorkspace={!workspaceRoot} />
      {!workspaceRoot ? (
        workspaceReady ? (
          <WorkspaceWelcome
            recents={recents ?? []}
            onOpenWorkspace={() => onOpenWorkspace?.()}
            onOpenRecent={(path) => onOpenRecent?.(path)}
          />
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

      <div ref={measureComposer} className="absolute inset-x-0 bottom-0 pointer-events-none z-20">
        <ComposerBar view={view} workspaceRoot={workspaceRoot} sessionKey={sessionKey} />
      </div>
        </>
      )}
    </>
  );
}

