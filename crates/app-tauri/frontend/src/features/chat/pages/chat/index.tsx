// Chat page — the conversation column of the main window: window title
// bar (with the git/usage meter), the scrollable stream, and the composer.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Plus } from "@keyline-icons/react";
import { toast } from "sonner";
import { TitleBar } from "@/components/title-bar";
import { ChatStream } from "@/features/chat/components/chat-stream";
import { SessionDock } from "@/features/chat/components/dock";
import { useDockStore } from "@/features/chat/components/dock/state";
import { ComposerBar } from "@/features/chat/components/composer-bar";
import { WorkspaceWelcome, type RecentWorkspace } from "@/features/chat/components/workspace-welcome";
import { useActiveView, useAgentChips, useAgentStore, useHasMoreHistory, useHasView, viewNow } from "@/stores/agent-store";
import { listBgJobs, openPath, saveArtifact, statPaths } from "@/lib/agent-ipc/index";
import { jobTone, summarizeJobs } from "@/lib/bg-jobs";
import { dockPill } from "@/lib/dock-pill";
import { isAppInFront } from "@/lib/desktop-notify";
import { useWorkspaceChanges } from "@/features/chat/hooks/use-workspace-changes";
import { extractRunningChildren } from "@/features/chat/hooks/running-children";
import type { BgJob } from "@/features/chat/hooks/stream-view";

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
  /** The same session's id, as a value. The key stays the identity the view
   * is cached under; parsing the id back out of it would be a second way to
   * say the same thing, and the two could drift. */
  sessionId?: number;
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
}

export function ChatPage({ title, workspaceRoot, officeMode, loading, sessionKey, sessionId = 0, onLoadOlder, onShowRaw, onOpenWorkspace, recents, onOpenRecent, workspaceReady = true, sessionsEmpty = false, onNewSession }: ChatPageProps) {
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

  // Background jobs: a view rebuilt from history has none until asked, and
  // the live feed only speaks on a change — seed the list once per session.
  // The view lands after the session pointer flips (the fold is async), so the
  // seed waits for it: a write into a session with no view is dropped, and this
  // effect would never run again.
  const hasView = useHasView(workspaceRoot, sessionId);
  const setBgJobs = useAgentStore((s) => s.setBgJobs);
  const writeJobs = useCallback(
    (jobs: BgJob[]) => setBgJobs(workspaceRoot, sessionId, jobs),
    [setBgJobs, workspaceRoot, sessionId],
  );
  // Failed jobs this view has already accounted for — see the toast below.
  // Declared here because the seed writes into it first.
  const seenFailuresRef = useRef<Set<string> | null>(null);
  useEffect(() => {
    if (!workspaceRoot || !sessionId || !hasView) return;
    let live = true;
    void listBgJobs(workspaceRoot, sessionId)
      .then((r) => {
        if (!live) return;
        // A live snapshot may have landed while the read was in flight — the
        // feed is newer than the persisted record, so it keeps the list.
        if (viewNow(workspaceRoot, sessionId)?.bgJobsAt) return;
        // The seed is history, not news: whatever already failed is on the
        // pill and must not toast when the list lands.
        const seen = seenFailuresRef.current ?? new Set<string>();
        for (const j of r.jobs) if (jobTone(j) === "failed") seen.add(j.id);
        seenFailuresRef.current = seen;
        writeJobs(r.jobs);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [workspaceRoot, sessionId, hasView, writeJobs]);
  const jobsSummary = useMemo(() => summarizeJobs(view.bgJobs), [view.bgJobs]);
  // Delegated children still out — the composer's own capsule names them.
  const runningChildren = useMemo(() => extractRunningChildren(view), [view]);

  // The dock is one sidebar for every list, so "has content" is the sum —
  // and which view to reveal follows whatever actually appeared.
  const dockOpen = useDockStore((s) => s.open);
  const dockTab = useDockStore((s) => s.tab);
  const setDockOpen = useDockStore((s) => s.setOpen);
  const setDockTab = useDockStore((s) => s.setTab);
  const filesTab = useDockStore((s) => s.filesTab);
  // 改动 is the WORKSPACE's uncommitted git files, not this session's write
  // log: a reopened session replays part of its history, so its folded list
  // shrinks while the tree it changed is still dirty. `repo: false` = a plain
  // folder has no git to ask, and the session's own list is all there is.
  const workspaceChanges = useWorkspaceChanges(workspaceRoot);
  const changes = workspaceChanges.repo ? workspaceChanges.files : view.changes;
  const filesCount = changes.length + artifacts.length;
  const panelCount = filesCount + view.bgJobs.length;
  // The dock's own open flag AND its content — an empty sidebar hides itself,
  // so a fresh session never lands on 「本会话还没有文件产出」 just because the
  // previous one left the dock open (the flag is persisted).
  const dockVisible = dockOpen && panelCount > 0;
  // The jobs pill speaks for the 后台 view; the dock pill for the file views.
  // Each reads as open only while the dock shows its own view, so the two
  // never both claim the sidebar. With no files the dock falls back to 后台
  // whatever tab is stored, so that counts as shown too.
  const jobsShown =
    dockVisible && view.bgJobs.length > 0 && (dockTab === "jobs" || filesCount === 0);
  // Which view the pill names: the last FILE view — 后台 is a detour, so the
  // label does not jump while it is up — if it still has content.
  const { tab: activeDockTab, count: activeDockCount } = dockPill({
    filesTab,
    changes: changes.length,
    artifacts: artifacts.length,
  });
  // The 改动 view's own numbers for the pill — same figures its header shows.
  const dockAdds = useMemo(
    () => changes.reduce((n, c) => n + c.adds, 0),
    [changes],
  );
  const dockDels = useMemo(
    () => changes.reduce((n, c) => n + c.dels, 0),
    [changes],
  );
  const dismissedRef = useRef(false);
  const autoOpenedRef = useRef(false);
  useEffect(() => {
    dismissedRef.current = false;
    autoOpenedRef.current = false;
  }, [sessionKey]);
  useEffect(() => {
    // Files only: a background job is not a deliverable, and a sidebar that
    // sprang open for every build would be noise. Its pill is the signal —
    // so this waits for a view the pill would actually name.
    if (activeDockCount === 0 || autoOpenedRef.current || dismissedRef.current || dockOpen) return;
    autoOpenedRef.current = true;
    // Show the list that just got its first item — a turn that writes files
    // and a turn that generates documents both land here.
    setDockTab(activeDockTab ?? "changes");
    setDockOpen(true);
  }, [activeDockCount, activeDockTab, dockOpen, setDockOpen, setDockTab]);
  // The dock pill toggles the file views; from 后台 it switches back to them
  // instead of closing a sidebar the user did not open for files.
  const dockFilesShown = dockVisible && !jobsShown;
  const handleToggleDock = useCallback(() => {
    if (dockFilesShown) {
      // Closing by hand is a decision, not a pause: it silences the auto-open.
      dismissedRef.current = true;
      setDockOpen(false);
      return;
    }
    if (activeDockTab) setDockTab(activeDockTab);
    setDockOpen(true);
  }, [dockFilesShown, activeDockTab, setDockOpen, setDockTab]);
  const handleToggleJobs = useCallback(() => {
    if (jobsShown) {
      dismissedRef.current = true;
      setDockOpen(false);
      return;
    }
    setDockTab("jobs");
    setDockOpen(true);
  }, [jobsShown, setDockOpen, setDockTab]);

  // A job that fails is worth a toast with a way to look — once per job, and
  // only for a failure that happened while this view was watching (a reopened
  // session's old failures are already on its pill). The toast is for the
  // user looking at this window: when it is not in front, the notification
  // rules (`notifyForEvent`) send the system notice instead.
  useEffect(() => {
    seenFailuresRef.current = null;
  }, [sessionKey]);
  useEffect(() => {
    // Restored rows are an earlier run's record, never a fresh failure.
    const failed = view.bgJobs.filter((j) => !j.restored && jobTone(j) === "failed");
    if (seenFailuresRef.current === null) {
      seenFailuresRef.current = new Set(failed.map((j) => j.id));
      return;
    }
    const seen = seenFailuresRef.current;
    for (const j of failed) {
      if (seen.has(j.id)) continue;
      seen.add(j.id);
      if (!isAppInFront()) continue;
      toast.error("后台任务失败", {
        description: j.command,
        duration: 10000,
        action: {
          label: "查看",
          onClick: () => {
            setDockTab("jobs");
            setDockOpen(true);
          },
        },
      });
    }
  }, [view.bgJobs, setDockOpen, setDockTab]);
  // The dock's own close button already flipped the flag — only the dismissal
  // bookkeeping is left here (toggling again would flip it straight back open).
  const handleDockClosed = useCallback(() => {
    dismissedRef.current = true;
  }, []);

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
              dockOpen={dockFilesShown}
              dockCount={activeDockCount}
              dockTab={activeDockTab}
              dockAdds={dockAdds}
              dockDels={dockDels}
              onToggleDock={workspaceRoot ? handleToggleDock : undefined}
              jobs={jobsSummary}
              jobsOpen={jobsShown}
              onToggleJobs={workspaceRoot ? handleToggleJobs : undefined}
              subagents={runningChildren}
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

          {/* The unified dock: 改动 + 产物 in one sidebar, either mode. It reads
              its own open/tab/width from the dock store; the page only feeds
              content and the manual-close dismissal. */}
          <SessionDock
            changes={changes}
            artifacts={artifacts}
            jobs={view.bgJobs}
            jobsAt={view.bgJobsAt}
            root={workspaceRoot}
            session={sessionId}
            onJobs={writeJobs}
            open={dockVisible}
            onClose={handleDockClosed}
          />
        </div>
      )}
    </>
  );
}


