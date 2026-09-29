// Chat page — the conversation column of the main window: window title
// bar (with the git/usage meter), the scrollable stream, and the composer.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Plus } from "@keyline-icons/react";
import { toast } from "sonner";
import { TitleBar } from "@/components/title-bar";
import { ChatStream } from "@/features/chat/components/chat-stream";
import { ChangesPanel } from "@/features/chat/components/changes-panel";
import { ArtifactsPanel } from "@/features/chat/components/artifacts-panel";
import { ComposerBar } from "@/features/chat/components/composer-bar";
import { WorkspaceWelcome, type RecentWorkspace } from "@/features/chat/components/workspace-welcome";
import { useActiveView, useAgentChips, useHasMoreHistory } from "@/stores/agent-store";
import { openPath, saveArtifact, statPaths } from "@/lib/agent-ipc/index";
import { cn } from "@/lib/utils";

/** Document extensions that pop the ready-toast — the office deliverable.
 *  Downloaded assets (images etc.) register silently in the dock. */
const DOC_EXTS = new Set(["pptx", "ppt", "docx", "doc", "xlsx", "xls", "pdf"]);

interface ChatPageProps {
  title: string;
  workspaceRoot: string;
  /** Active workspace is the office pseudo-workspace — the composer's
   *  mode dropdown only offers「工作」in this state. */
  officeMode?: boolean;
  /** Session/workspace switch in flight — the stream shows a skeleton. */
  loading?: boolean;
  /** Active session key (`root:id`) — resets the stream's incremental
   * mount window on session AND workspace switches. */
  sessionKey?: string;
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
  /** Session list resolved and the workspace has ZERO sessions — show the
   *  empty state instead of a composer that would send to nothing. */
  sessionsEmpty?: boolean;
  /** The empty state's CTA — creates the workspace's next session. */
  onNewSession?: () => void;
  /** Changes panel open state + toggle, owned by the app shell. */
  changesOpen?: boolean;
  onToggleChanges?: () => void;
}

export function ChatPage({ title, workspaceRoot, officeMode, loading, sessionKey, onLoadOlder, onShowRaw, onOpenWorkspace, recents, onOpenRecent, workspaceReady = true, sessionsEmpty = false, onNewSession, changesOpen, onToggleChanges }: ChatPageProps) {
  // The conversation's live data comes from the store, not from props: the
  // shell must not be a subscriber of the 60 fps stream (it used to re-render —
  // and drag the sidebar with it — for a whole turn).
  const view = useActiveView();
  const { gitInfo, ctxWindow: contextWindowHint, modelCost } = useAgentChips();
  const hasMore = useHasMoreHistory();
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

  // Ghost-filter — artifact rows register from tool args, so a file the
  // session later renamed or deleted (`web_download` wrote x.png → `mv`
  // made it .jpg) would still list. Stat the paths and drop the missing;
  // the filtered list feeds the dock, the title-bar count and the toast.
  const [missingArtifacts, setMissingArtifacts] = useState<Set<string>>(new Set());
  const artifactRels = view.artifacts.map((a) => a.path).join("\n");
  useEffect(() => {
    const paths = artifactRels.split("\n").filter(Boolean);
    if (paths.length === 0) {
      setMissingArtifacts(new Set());
      return;
    }
    let live = true;
    void statPaths(paths)
      .then((res) => {
        if (!live) return;
        setMissingArtifacts(new Set(paths.filter((p) => res[p] !== true)));
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [artifactRels]);
  const artifacts = useMemo(
    () => view.artifacts.filter((a) => !missingArtifacts.has(a.path)),
    [view.artifacts, missingArtifacts],
  );

  // Auto-open the panel once per session on the first file change; a
  // manual close (dismissedRef) suppresses the auto-open afterwards.
  const panelCount = officeMode ? artifacts.length : view.changes.length;
  // `changesOpen` alone isn't enough — an empty list hides the panel too.
  const changesVisible = !!changesOpen && panelCount > 0;
  const dismissedRef = useRef(false);
  const autoOpenedRef = useRef(false);
  useEffect(() => {
    dismissedRef.current = false;
    autoOpenedRef.current = false;
  }, [sessionKey]);
  useEffect(() => {
    if (panelCount > 0 && !autoOpenedRef.current && !dismissedRef.current && !changesOpen) {
      autoOpenedRef.current = true;
      onToggleChanges?.();
    }
  }, [panelCount, changesOpen, onToggleChanges]);
  const handleToggleChanges = useCallback(() => {
    if (changesOpen) dismissedRef.current = true;
    else dismissedRef.current = false;
    onToggleChanges?.();
  }, [changesOpen, onToggleChanges]);

  const seenArtifactsRef = useRef<Set<string>>(new Set());
  useEffect(() => {
    seenArtifactsRef.current = new Set(artifacts.map((a) => a.path));
  }, [sessionKey]);
  useEffect(() => {
    const seen = seenArtifactsRef.current;
    if (!view.turnOpen) {
      artifacts.forEach((a) => seen.add(a.path));
      return;
    }
    for (const a of artifacts) {
      if (seen.has(a.path)) continue;
      seen.add(a.path);
      if (!DOC_EXTS.has(a.ext)) continue;
      toast.success("产物已生成", {
        description: a.path,
        duration: 15000,
        action: {
          label: "打开",
          onClick: () => void openPath(a.path).catch((e) => toast.error(`打开失败：${e}`)),
        },
        cancel: {
          label: "另存为",
          onClick: () =>
            void saveArtifact(a.path)
              .then((r) => {
                if (r.saved) toast.success("已另存", { description: r.path });
              })
              .catch((e) => toast.error(`另存失败：${e}`)),
        },
      });
    }
  }, [artifacts, view.turnOpen]);

  return (
    <>
      <TitleBar
        title={title}
        view={view}
        gitInfo={gitInfo}
        contextWindowHint={contextWindowHint}
        modelCost={modelCost}
        onShowRaw={onShowRaw}
        noWorkspace={!workspaceRoot}
        // Zero sessions — session-scoped chips (ctx/tokens/cost/rate) would
        // only show config defaults; git is workspace-level and stays.
        noSession={sessionsEmpty}
        changesOpen={changesVisible}
        changesCount={panelCount}
        changesLabel={officeMode ? "产物" : "改动"}
        onToggleChanges={workspaceRoot ? handleToggleChanges : undefined}
      />
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
      ) : sessionsEmpty ? (
        // Every session deleted — the welcome layout with its CTA swapped
        // to 「新建会话」: greeting, activity stats, capability cards and
        // composer sigils all come along unchanged. Recents stay off —
        // 「最近打开」belongs to the no-workspace screen; the current
        // project is already open here.
        <WorkspaceWelcome
          recents={[]}
          onOpenWorkspace={() => onOpenWorkspace?.()}
          onOpenRecent={(path) => onOpenRecent?.(path)}
          office={officeMode}
          cta={{
            Icon: Plus,
            label: "新建会话",
            onClick: () => onNewSession?.(),
            hint: "或从左侧「会话」列表新建",
          }}
        />
      ) : (
        // Stream + changes dock side by side below the title bar — the
        // title bar spans the full width, so the panel can't cover the
        // window controls. The dock's wrapper animates its width so the
        // stream re-flows smoothly instead of jumping.
        <div className="flex flex-1 min-h-0">
          <div className="relative flex-1 min-w-0 flex flex-col transition-[flex-basis,margin] duration-300 ease-out">
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
              <ComposerBar view={view} workspaceRoot={workspaceRoot} sessionKey={sessionKey} officeMode={officeMode} />
            </div>
          </div>

          {/* Dock wrapper animates its width so the stream gives way
              smoothly; the inner panel slides in from the right edge. */}
          <div
            className={cn(
              "flex-none overflow-hidden transition-[width] duration-300 ease-out",
              changesVisible ? "w-[300px]" : "w-0",
            )}
            aria-hidden={!changesVisible}
          >
            <div
              className={cn(
                "h-full w-[300px] transition-transform duration-300 ease-out",
                changesVisible ? "translate-x-0" : "translate-x-full",
              )}
            >
              {officeMode ? (
                <ArtifactsPanel artifacts={view.artifacts} onClose={handleToggleChanges} />
              ) : (
                <ChangesPanel changes={view.changes} onClose={handleToggleChanges} />
              )}
            </div>
          </div>
        </div>
      )}
    </>
  );
}


