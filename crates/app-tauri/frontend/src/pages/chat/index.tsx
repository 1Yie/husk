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
}

export function ChatPage({ title, view, workspaceRoot, gitInfo, contextWindowHint }: ChatPageProps) {
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
      <TitleBar title={title} view={view} gitInfo={gitInfo} contextWindowHint={contextWindowHint} />
      <ChatStream view={view} bottomPad={composerH + 24} composerH={composerH} />

      {/* Bottom gradient mask: subtle, soft dissolve behind the floating composer */}
      <div
        style={{ height: composerH + 16 }}
        className="absolute inset-x-0 bottom-0 pointer-events-none z-10 bg-gradient-to-t from-white/75 via-white/35 to-transparent dark:from-[#141416]/75 dark:via-[#141416]/35 dark:to-transparent"
        aria-hidden="true"
      />

      <div ref={composerRef} className="absolute inset-x-0 bottom-0 pointer-events-none z-20">
        <ComposerBar view={view} workspaceRoot={workspaceRoot} />
      </div>
    </>
  );
}
