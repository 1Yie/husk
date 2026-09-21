// Chat page — the conversation column of the main window: window title
// bar (with the git/usage meter), the scrollable stream, and the composer.
import { useEffect, useRef, useState } from "react";
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
}

export function ChatPage({ title, view, workspaceRoot, gitInfo, contextWindowHint, modelCost, loading, sessionKey, hasMore, onLoadOlder, onShowRaw }: ChatPageProps) {
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
      <TitleBar title={title} view={view} gitInfo={gitInfo} contextWindowHint={contextWindowHint} modelCost={modelCost} onShowRaw={onShowRaw} />
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
  );
}
