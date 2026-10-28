// 「记忆」pane — the read/write side of the three memory write paths
// (remember tool, /remember command, turn-end distillation). Persona is
// key/value with a global or a workspace scope; facts + episodes are always
// this workspace's.

import { SkeletonKvRows } from "@/components/ui/skeleton";
import { useDelayedFlag } from "@/hooks/use-delayed-flag";
import { toast } from "sonner";
import { useEffect, useId, useState } from "react";
import {
  Bin,
  Brain,
  ChevronDown,
  ChevronUp,
  CircleAlert,
  CircleUser,
  Clock,
  Database,
  Folder,
  Globe,
  Plus,
  Radar,
  Sparkles,
  SquarePen,
} from "@keyline-icons/react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import { Switch } from "@/components/ui/switch";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { KvList, KvListContent } from "@/components/ui/kv-list";
import { AnimatedHeight } from "@/components/ui/animated-height";
import { SettingSelect, SettingsSection } from "@/features/settings/components";
import { DeleteButton } from "@/features/settings/components/delete-button";
import { FormDialog } from "@/features/settings/components/form-dialog";
import {
  EmptyState,
  Field,
  ItemDetail,
  ItemIcon,
  ItemRow,
  ScopeTabs,
  SearchBox,
  Tag,
  matchQuery,
} from "@/features/settings/pages/agent/shared/index";
import { timeAgo } from "@/lib/time";
import { cn } from "@/lib/utils";
import {
  cancelSemanticRecall,
  clearMemory,
  forgetMemory,
  getMemoryOverview,
  getSemanticProgress,
  removeMemoryFact,
  removeSemanticModel,
  retrySemanticRecall,
  setDefaultPrefs,
  setMemoryPersona,
  type MemoryOverview,
  type PersonaEntry,
  type SemanticProgress,
} from "@/lib/agent-ipc/sessions";

/** Where a preference applies. */
type Scope = "global" | "workspace";

/** The pane's own view of a row — the wire omits `scope` on older payloads,
 *  and those entries are global. */
type PersonaRow = PersonaEntry & { scope: Scope };

/** The 个人偏好 filter: 全部 plus one tab per scope. */
type ScopeFilter = "all" | Scope;

/** Icon tint per scope — the glyph alone is easy to miss in a list of twelve. */
const SCOPE_ICON_TONE: Record<Scope, string> = {
  global: "text-sky-600 dark:text-sky-300",
  workspace: "text-violet-600 dark:text-violet-300",
};

/** Confidence → tag tone: the distiller's certainty at a glance. */
const confidenceTone = (c: number) => (c >= 0.8 ? "success" : c >= 0.5 ? "neutral" : "warn");

/** Bytes → whole megabytes, the unit the semantic row reports in. */
const toMb = (bytes: number) => Math.round(bytes / (1024 * 1024));

// Shared with sidebar paging: fast requests must not flash a busy indicator.

const PREVIEW_COUNT = 5;

/** Rows past the preview fade in as the list grows, staggered within their
 *  batch; preview rows render plain so the first paint never animates. */
const revealClass = (index: number) =>
  index < PREVIEW_COUNT
    ? undefined
    : cn(
        "animate-in fade-in-0 slide-in-from-top-1 fill-mode-both duration-300 motion-reduce:animate-none",
        [
          "[animation-delay:0ms]",
          "[animation-delay:40ms]",
          "[animation-delay:80ms]",
          "[animation-delay:120ms]",
          "[animation-delay:160ms]",
        ][index % PREVIEW_COUNT],
      );

function MemoryListFooter({ label, listId, total, visible, onMore, onCollapse }: {
  label: string;
  listId: string;
  total: number;
  visible: number;
  onMore: () => void;
  onCollapse: () => void;
}) {
  if (total <= PREVIEW_COUNT) return null;
  return (
    <div className="flex flex-wrap items-center justify-between gap-2 border-t border-hairline px-5 py-2.5">
      <span className="text-[11px] tabular-nums text-neutral-500">已显示 {visible} / {total} 条</span>
      <div className="ml-auto flex items-center gap-1">
        {visible > PREVIEW_COUNT && (
          <Button variant="ghost" size="sm" className="h-7 gap-1 text-xs text-neutral-500"
            aria-label={`收起${label}`} aria-controls={listId} onClick={onCollapse}>
            <ChevronUp className="h-3.5 w-3.5" /> 收起
          </Button>
        )}
        {visible < total && (
          <Button variant="ghost" size="sm" className="h-7 gap-1 text-xs text-neutral-600"
            aria-label={`显示更多${label}`} aria-controls={listId} onClick={onMore}>
            显示更多 <span className="text-[11px] tabular-nums text-neutral-500">剩余 {total - visible} 条</span>
            <ChevronDown className="h-3.5 w-3.5" />
          </Button>
        )}
      </div>
    </div>
  );
}

export function MemoryPane() {
  const [ov, setOv] = useState<MemoryOverview | null>(null);
  const [open, setOpen] = useState(false);
  /** The preference being edited — saving overwrites it, so the key stays locked. */
  const [edit, setEdit] = useState<PersonaRow | null>(null);
  const [newKey, setNewKey] = useState("");
  const [newVal, setNewVal] = useState("");
  /** Where the entry lands — 全局 unless the 工作区 filter says otherwise. */
  const [newScope, setNewScope] = useState<Scope>("global");
  /** The 个人偏好 filter; the list and its counts both follow it. */
  const [personaScope, setPersonaScope] = useState<ScopeFilter>("all");
  const [busy, setBusy] = useState(false);
  const [factQuery, setFactQuery] = useState("");
  const [personaLimit, setPersonaLimit] = useState(PREVIEW_COUNT);
  const [factLimit, setFactLimit] = useState(PREVIEW_COUNT);
  /** The download prompt — the only modal this feature has. Progress lives in
   *  the row, so a modal and the row can never disagree about it. */
  const [askSemantic, setAskSemantic] = useState(false);
  const [progress, setProgress] = useState<SemanticProgress | null>(null);
  /** The fetch ended badly. Its own flag rather than a test on `progress`, so
   *  the ticker's dependencies stay about the switch instead of every sample. */
  const [semanticFailed, setSemanticFailed] = useState(false);
  /** The encoder is in this process's memory. The backend keeps it through a
   *  switch-off, so switching back on is instant; only 卸载 clears it. */
  const [modelLoaded, setModelLoaded] = useState(false);
  const personaListId = useId();
  const factListId = useId();

  const reload = () => {
    void getMemoryOverview()
      .then((o) => {
        setOv(o);
        if (o.semantic_ready) setModelLoaded(true);
      })
      .catch(() => {});
  };
  useEffect(reload, []);

  /** Persist a switch. A failed write is reported AND rolled back: a toggle
   *  still showing a state the host never took is worse than the error. */
  const savePref = (
    patch: Parameters<typeof setDefaultPrefs>[0],
    rollback?: () => void,
  ) =>
    setDefaultPrefs(patch).catch((e) => {
      rollback?.();
      toast.error("保存失败", { description: String(e) });
    });

  /** A failed fetch is not permanent: every file that verified is reused, so a
   *  retry that only has to rebuild the session costs a second. */
  const retrySemantic = () => {
    setProgress(null);
    setSemanticFailed(false);
    void retrySemanticRecall()
      .then(setProgress)
      .catch((e) => toast.error("重试失败", { description: String(e) }));
  };

  /** Turn the switch on. A model already on disk needs no download and gets no
   *  prompt — asking again is what made "off, then on" read as "you uninstalled
   *  it". Only a machine that has never fetched the weights is asked. A model
   *  still in memory is ready at once, so nothing is polled for it. */
  const enableSemantic = () => {
    if (!ov) return;
    const before = { semantic: ov.semantic, semantic_ready: ov.semantic_ready };
    setSemanticFailed(false);
    setProgress(null);
    setOv({ ...ov, semantic: true, semantic_ready: modelLoaded });
    void savePref({ semantic_recall: true }, () =>
      setOv((o) => (o ? { ...o, ...before } : o)),
    );
  };

  /** 取消 during the fetch. The partial file stays backend-side and resumes on the
   *  next enable, so the row simply goes back to off. */
  const cancelSemantic = () => {
    if (!ov) return;
    void cancelSemanticRecall().catch((e) => toast.error("取消失败", { description: String(e) }));
    setSemanticFailed(false);
    setOv({ ...ov, semantic: false });
    void savePref({ semantic_recall: false }, () =>
      setOv((o) => (o ? { ...o, semantic: true } : o)),
    );
  };

  /** 卸载 — drop the weights and switch the feature off with them. */
  const uninstallSemantic = () => {
    if (!ov) return;
    setProgress(null);
    setSemanticFailed(false);
    setModelLoaded(false);
    setOv({ ...ov, semantic: false, semantic_ready: false, semantic_cached: false });
    void removeSemanticModel().catch(() => {});
    void savePref({ semantic_recall: false });
  };

  /** Whether anything is still waiting on the model. One ticker serves both
   *  consumers — the open dialog, and the row that must stop showing the fetch
   *  once it lands or fails. Two polls only doubled the IPC. */
  const watchingModel =
    !!ov?.enabled &&
    !!ov?.semantic &&
    !semanticFailed &&
    !ov.semantic_ready;
  useEffect(() => {
    if (!watchingModel) return;
    let live = true;
    const tick = () =>
      void getSemanticProgress()
        .then((p) => {
          if (!live) return;
          // Keep the previous object when the sample is unchanged: a poll that
          // moves nothing must not re-render the pane (and the spinner with it).
          setProgress((prev) =>
            prev && prev.phase === p.phase && prev.bytes === p.bytes && prev.file === p.file
              ? prev
              : p,
          );
          if (p.phase === "ready") {
            // The weights are on disk and in memory from here on. Recording that
            // is what keeps a later switch-off/on from offering a download that
            // no longer exists, and what keeps 卸载 on screen while the feature is off.
            setModelLoaded(true);
            setOv((o) =>
              o ? { ...o, semantic_ready: true, semantic_cached: true } : o,
            );
          } else if (p.phase === "failed") {
            // Stop polling: the reason is already on screen, and asking again
            // every second does not fix a blocked download.
            setSemanticFailed(true);
          }
        })
        .catch(() => {});
    tick();
    const timer = setInterval(tick, 800);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [watchingModel]);

  // Share of the model on disk. `total` comes from the backend's file
  // manifest, so this is a real fraction rather than an estimate.
  const semanticPct =
    progress && progress.total > 0
      ? Math.min(100, Math.round((progress.bytes / progress.total) * 100))
      : 0;

  // On, but nothing to use yet: the weights are still arriving or being read back.
  // The row shows the fetch or the load, and the switch waits on the load.
  const semanticBusy = !!ov?.semantic && !ov.semantic_ready && !semanticFailed;
  // A download only while bytes are arriving. A model already on disk that is
  // being read back into memory is loading, and must never read as a download.
  const semanticDownloading =
    semanticBusy &&
    (progress?.phase === "downloading" ||
      (progress?.phase !== "loading" && !ov?.semantic_cached));
  // The busy hint waits out a fast load, so a switch that settles at once shows nothing.
  const busyShown = useDelayedFlag(semanticBusy);

  const clearOne = async (table: string, label: string) => {
    try {
      const r = await clearMemory(table);
      toast.success(`已清空${label}`, { description: `移除 ${r.removed} 条` });
      reload();
    } catch (e) {
      toast.error("清空失败", { description: String(e) });
    }
  };

  const startAdd = () => {
    setEdit(null);
    setNewKey("");
    setNewVal("");
    // The filter already states the intent: adding while 工作区 is selected
    // must not need the scope picked a second time.
    setNewScope(personaScope === "workspace" ? "workspace" : "global");
    setOpen(true);
  };

  const startEdit = (p: PersonaEntry) => {
    setEdit({ ...p, scope: p.scope ?? "global" });
    setNewKey(p.key);
    setNewVal(p.value);
    setNewScope(p.scope ?? "global");
    setOpen(true);
  };

  /** Write the entry in `scope`. A changed scope is a move, and only the row it
   *  came from is forgotten — the label may be a separate entry in the other
   *  scope, and dropping that one would delete a preference the user never
   *  touched. It is written back if the re-write fails: a move must not end
   *  with the preference gone. */
  const writePersona = async (key: string, value: string, scope: Scope) => {
    if (!edit || edit.scope === scope) {
      await setMemoryPersona(key, value, scope);
      return;
    }
    const from = edit;
    await forgetMemory(key, from.scope);
    try {
      await setMemoryPersona(key, value, scope);
    } catch (e) {
      await setMemoryPersona(key, from.value, from.scope).catch(() => {});
      throw e;
    }
  };

  const savePersona = async () => {
    const k = newKey.trim();
    const v = newVal.trim();
    if (!k || !v) return;
    setBusy(true);
    try {
      await writePersona(k, v, newScope);
      toast.success(edit ? "已更新偏好" : "已记住偏好");
      setOpen(false);
      setEdit(null);
      setNewKey("");
      setNewVal("");
      reload();
    } catch (e) {
      toast.error("保存失败", { description: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const persona = ov?.persona ?? [];
  const facts = ov?.facts ?? [];
  /** Old payloads carry no scope, and everything written before scoping was global. */
  const scopeOf = (p: PersonaEntry): Scope => p.scope ?? "global";
  const scopedPersona = persona.filter(
    (p) => personaScope === "all" || scopeOf(p) === personaScope,
  );
  const scopeCounts = {
    all: persona.length,
    global: persona.filter((p) => scopeOf(p) === "global").length,
    workspace: persona.filter((p) => scopeOf(p) === "workspace").length,
  };
  const shownPersona = scopedPersona.slice(0, personaLimit);
  const filteredFacts = facts.filter((f) => matchQuery(factQuery, f.text));
  const shownFacts = filteredFacts.slice(0, factLimit);

  /** Lists stay readable while memory is off, just visibly inactive. */
  const inactive = !!ov && !ov.enabled;

  const count = (n: number | undefined) => (
    <span className="text-[11px] tabular-nums text-neutral-400">{n ?? "–"} 条</span>
  );

  return (
    <div className="flex flex-col gap-8 w-full">
      <SettingsSection title="记忆开关">
        <KvList>
          <KvListContent>
            <ItemRow
              icon={
                <ItemIcon status={ov ? (ov.enabled ? "success" : "idle") : undefined}>
                  <Brain className="h-4 w-4" />
                </ItemIcon>
              }
              title="启用记忆"
              description="跨会话保留偏好与工作区事实，并在对话中自动召回"
              actions={
                <Switch
                  checked={ov?.enabled ?? true}
                  onCheckedChange={(v) => {
                    if (!ov) return;
                    setOv({ ...ov, enabled: v });
                    void savePref({ memory_enabled: v }, () =>
                      setOv((o) => (o ? { ...o, enabled: !v } : o)),
                    );
                  }}
                />
              }
            />
            <ItemRow
              className={cn(inactive && "opacity-60")}
              icon={
                <ItemIcon>
                  <Sparkles className="h-4 w-4" />
                </ItemIcon>
              }
              title="模型蒸馏"
              description="每轮结束后由模型提取任务摘要、事实与偏好"
              actions={
                <Switch
                  checked={ov?.distill ?? true}
                  onCheckedChange={(v) => {
                    if (!ov) return;
                    setOv({ ...ov, distill: v });
                    void savePref({ memory_distill: v }, () =>
                      setOv((o) => (o ? { ...o, distill: !v } : o)),
                    );
                  }}
                />
              }
            />
            {/* Off by default; the model behind it is a download, so turning it
             *  ON asks first. Turning it off is immediate. A build without the
             *  encoder gets no row at all — a switch that cannot do anything.
             *
             *  While the fetch runs the row shows its progress and 取消 takes the
             *  switch's place. Once the weights are on disk but still being read
             *  back, the switch is LOCKED: it is on, but there is nothing to use yet. */}
            {ov?.semantic_available && (
              <div>
                <ItemRow
                  className={cn(inactive && "opacity-60")}
                  icon={
                    <ItemIcon
                      status={
                        ov.semantic && ov.semantic_ready
                          ? "success"
                          : semanticFailed
                            ? "danger"
                            : ov.semantic
                              ? "idle"
                              : undefined
                      }
                    >
                      <Radar className="h-4 w-4" />
                    </ItemIcon>
                  }
                  title="语义召回"
                  tags={semanticFailed ? <Tag tone="danger">下载失败</Tag> : undefined}
                  description={
                    inactive ? (
                      "记忆关闭时不生效"
                    ) : !ov.semantic ? (
                      "本地小型语义模型，给记忆召回打分。"
                    ) : ov.semantic_ready ? (
                      `${ov.semantic_model ?? "本地语义模型"} · 本地运行`
                    ) : semanticFailed ? (
                      <span className="text-red-600 dark:text-red-400">
                        {progress?.error ?? "下载失败，召回暂时按关键词匹配"}
                      </span>
                    ) : semanticDownloading ? (
                      "本地模型下载完成后自动启用"
                    ) : undefined
                  }
                  actions={
                    <>
                      {/* 卸载 belongs to the weights: there is nothing to delete
                       *  while they are still arriving, and a bin next to a
                       *  running download reads as "cancel". */}
                      {(ov.semantic_cached || ov.semantic_ready) && (
                        <DeleteButton
                          title="卸载本地模型"
                          confirmTitle="卸载本地模型"
                          confirmDescription="将删除已下载的语义模型文件，并关闭语义召回。下次启用需要重新下载。"
                          confirmLabel="卸载"
                          onConfirm={uninstallSemantic}
                        />
                      )}
                      {semanticFailed && (
                        <Button variant="outline" size="sm" className="h-7 text-xs" onClick={retrySemantic}>
                          重试
                        </Button>
                      )}
                      {semanticDownloading ? (
                        <Button variant="ghost" size="sm" className="h-7 text-xs" onClick={cancelSemantic}>
                          取消
                        </Button>
                      ) : (
                        <Switch
                          // A failed model is not in effect, so the switch must
                          // not claim it is — the preference is still on file,
                          // which is why the next launch tries once more.
                          checked={ov.semantic && !semanticFailed}
                          // Locked while the weights are being read back: it is
                          // on, but there is nothing to use yet, so it is not a choice.
                          disabled={inactive || busyShown}
                          aria-label="语义召回"
                          onCheckedChange={(v) => {
                            if (v) {
                              // Already agreed to the download once: this is a
                              // retry, not a fresh decision.
                              if (semanticFailed) {
                                retrySemantic();
                                return;
                              }
                              // The weights are on disk, or already in memory —
                              // nothing to warn about, so the switch just works.
                              if (ov.semantic_cached || modelLoaded) {
                                enableSemantic();
                                return;
                              }
                              setAskSemantic(true);
                              return;
                            }
                            // Switching off keeps `modelLoaded`: the encoder stays
                            // in memory, so switching back on is instant.
                            setSemanticFailed(false);
                            setOv({ ...ov, semantic: false });
                            void savePref({ semantic_recall: false }, () =>
                              setOv((o) => (o ? { ...o, semantic: true } : o)),
                            );
                          }}
                        />
                      )}
                    </>
                  }
                />
                {busyShown && (
                  <ItemDetail>
                    {semanticDownloading ? (
                      <>
                        <Progress value={progress && progress.total > 0 ? semanticPct : undefined} />
                        <span className="flex items-center justify-between text-[11px] tabular-nums text-neutral-500">
                          <span>
                            {progress && progress.total > 0 &&
                              `已下载 ${toMb(progress.bytes)} MB / ${toMb(progress.total)} MB`}
                          </span>
                          <span role="status" aria-live="polite">{semanticPct}%</span>
                        </span>
                      </>
                    ) : (
                      <>
                        <Progress />
                        <span className="text-[11px] text-neutral-500">正在加载模型…</span>
                      </>
                    )}
                  </ItemDetail>
                )}
              </div>
            )}
          </KvListContent>
        </KvList>
      </SettingsSection>

      {ov?.error && (
        <div className="flex items-start gap-2.5 rounded-xl border border-red-200 bg-red-50 px-4 py-3 text-xs text-red-700 dark:border-red-500/30 dark:bg-red-500/10 dark:text-red-300">
          <CircleAlert className="mt-px h-3.5 w-3.5 shrink-0" />
          <div className="flex min-w-0 flex-col gap-0.5">
            <span className="font-medium">记忆库无法打开</span>
            <span className="break-all font-mono text-[11px] opacity-80">{ov.error}</span>
            <span className="opacity-80">下方显示为空并非没有记忆</span>
          </div>
        </div>
      )}

      <SettingsSection
        title="个人偏好"
        description="全局或仅当前工作区，每轮对话都会附带"
        actions={
          <>
            <ScopeTabs
              value={personaScope}
              onChange={(v) => {
                setPersonaScope(v);
                // A narrower filter starts at the top: a list already expanded
                // to twelve rows would hide the point of switching.
                setPersonaLimit(PREVIEW_COUNT);
              }}
              options={[
                { value: "all", label: "全部", count: ov ? scopeCounts.all : undefined },
                { value: "global", label: "全局", count: ov ? scopeCounts.global : undefined },
                { value: "workspace", label: "工作区", count: ov ? scopeCounts.workspace : undefined },
              ]}
            />
            <Button size="sm" variant="outline" className="h-7 text-xs gap-1" onClick={startAdd}>
              <Plus className="h-3.5 w-3.5" /> 添加
            </Button>
          </>
        }
      >
        <KvList className={cn(inactive && "opacity-60")}>
          <AnimatedHeight>
          <KvListContent id={personaListId}>
            {!ov ? (
              <SkeletonKvRows rows={2} />
            ) : scopedPersona.length ? (
              shownPersona.map((p, i) => (
                <ItemRow
                  key={`${p.scope}:${p.key}`}
                  className={revealClass(i)}
                  icon={
                    <ItemIcon className={SCOPE_ICON_TONE[scopeOf(p)]}>
                      {p.scope === "workspace" ? (
                        <Folder className="h-4 w-4" />
                      ) : (
                        <Globe className="h-4 w-4" />
                      )}
                    </ItemIcon>
                  }
                  title={p.key}
                  monoTitle
                  tags={<Tag>{p.scope === "workspace" ? "工作区" : "全局"}</Tag>}
                  description={p.value}
                  actions={
                    <>
                      <Button variant="ghost" size="icon" className="h-6 w-6" title="编辑"
                        onClick={() => startEdit(p)}>
                        <SquarePen className="h-3.5 w-3.5" />
                      </Button>
                      <DeleteButton
                        confirmTitle="删除偏好"
                        confirmDescription={`确定要删除偏好「${p.key}」吗？此操作无法撤销。`}
                        onConfirm={() =>
                          // Only the row on screen: the same label may be kept in
                          // the other scope, and that entry is not what was clicked.
                          void forgetMemory(p.key, scopeOf(p))
                            .then(() => {
                              toast.success("已删除偏好");
                              reload();
                            })
                            .catch((e) => toast.error("删除失败", { description: String(e) }))
                        }
                      />
                    </>
                  }
                />
              ))
            ) : (
              <EmptyState
                icon={<CircleUser className="h-4 w-4" />}
                title={
                  personaScope === "workspace"
                    ? "工作区暂无偏好"
                    : personaScope === "global"
                      ? "全局暂无偏好"
                      : "暂无偏好"
                }
                description={
                  personaScope === "all"
                    ? "对话中说「记住…」或在此手动添加，例如回复语言、代码风格"
                    : "切换范围查看其它偏好"
                }
                action={
                  <Button size="sm" variant="outline" className="h-7 text-xs gap-1" onClick={startAdd}>
                    <Plus className="h-3.5 w-3.5" /> 添加偏好
                  </Button>
                }
              />
            )}
          </KvListContent>
          </AnimatedHeight>
          <MemoryListFooter
            label="个人偏好"
            listId={personaListId}
            total={scopedPersona.length}
            visible={shownPersona.length}
            onMore={() => setPersonaLimit((n) => n + PREVIEW_COUNT)}
            onCollapse={() => setPersonaLimit(PREVIEW_COUNT)}
          />
        </KvList>
      </SettingsSection>

      {/* ---------- 记忆事实（当前工作区） ---------- */}
      <SettingsSection
        title={
          <span className="flex items-center gap-2">
            记忆事实
            <Tag icon={<Folder className="h-2.5 w-2.5" />}>工作区</Tag>
          </span>
        }
        description="只在本工作区生效，由蒸馏或 remember 工具写入"
        actions={
          <>
            {count(ov ? facts.length : undefined)}
            {/* Search shares the header row — a row of its own left a band of
             *  empty space between the description and the list. */}
            {(facts.length > PREVIEW_COUNT || factQuery !== "") && (
              <SearchBox value={factQuery} onChange={(value) => {
                setFactQuery(value);
                setFactLimit(PREVIEW_COUNT);
              }} placeholder="搜索事实" />
            )}
          </>
        }
      >
        <KvList className={cn(inactive && "opacity-60")}>
          <AnimatedHeight>
          <KvListContent id={factListId}>
            {!ov ? (
              <SkeletonKvRows rows={3} />
            ) : shownFacts.length ? (
              shownFacts.map((f, i) => (
                <ItemRow
                  key={f.id}
                  className={revealClass(i)}
                  icon={
                    <ItemIcon
                      badge={{
                        label: `${Math.round(f.confidence * 100)}%`,
                        tone: confidenceTone(f.confidence),
                      }}
                    >
                      <Database className="h-4 w-4" />
                    </ItemIcon>
                  }
                  title={f.text}
                  wrapTitle
                  description={f.created_at ? `记录于 ${timeAgo(f.created_at)}` : undefined}
                  actions={
                    <DeleteButton
                      confirmTitle="删除记忆事实"
                      confirmDescription="确定要删除这条记忆事实吗？此操作无法撤销。"
                      onConfirm={() =>
                        void removeMemoryFact(f.id)
                          .then(() => {
                            toast.success("已删除事实");
                            reload();
                          })
                          .catch((e) => toast.error("删除失败", { description: String(e) }))
                      }
                    />
                  }
                />
              ))
            ) : (
              <EmptyState
                icon={<Database className="h-4 w-4" />}
                title={factQuery.trim() ? "没有匹配的事实" : "暂无事实"}
                description={
                  factQuery.trim()
                    ? "换个关键词试试"
                    : "开启模型蒸馏后，对话中的项目约定与关键结论会沉淀在这里"
                }
              />
            )}
          </KvListContent>
          </AnimatedHeight>
          <MemoryListFooter
            label="记忆事实"
            listId={factListId}
            total={filteredFacts.length}
            visible={shownFacts.length}
            onMore={() => setFactLimit((n) => n + PREVIEW_COUNT)}
            onCollapse={() => setFactLimit(PREVIEW_COUNT)}
          />
        </KvList>
      </SettingsSection>

      {/* ---------- 数据 ---------- */}
      <SettingsSection title="数据">
        <KvList>
          <KvListContent>
            <ItemRow
              icon={
                <ItemIcon>
                  <Database className="h-4 w-4" />
                </ItemIcon>
              }
              title="记忆事实"
              description={`当前工作区 ${ov?.facts.length ?? 0} 条`}
              actions={
                <DeleteButton
                  variant="text"
                  label="清空"
                  confirmTitle="清空记忆事实"
                  confirmDescription="确定要清空全部记忆事实吗？此操作无法撤销。"
                  confirmLabel="清空"
                  onConfirm={() => void clearOne("facts", "记忆事实")}
                />
              }
            />
            <ItemRow
              icon={
                <ItemIcon>
                  <Clock className="h-4 w-4" />
                </ItemIcon>
              }
              title="轮次经历"
              description={`当前工作区 ${ov?.episodes ?? 0} 条`}
              actions={
                <DeleteButton
                  variant="text"
                  label="清空"
                  confirmTitle="清空轮次经历"
                  confirmDescription="确定要清空全部轮次经历吗？此操作无法撤销。"
                  confirmLabel="清空"
                  onConfirm={() => void clearOne("episodes", "轮次经历")}
                />
              }
            />
            <ItemRow
              icon={
                <ItemIcon>
                  <Bin className="h-4 w-4" />
                </ItemIcon>
              }
              title="全部记忆"
              description="偏好、事实与经历一并清除，不可恢复"
              actions={
                <DeleteButton
                  variant="text"
                  label="全部清空"
                  confirmTitle="清空全部记忆"
                  confirmDescription="确定要清空全部记忆吗？偏好、事实与经历将一并清除，此操作无法撤销。"
                  confirmLabel="全部清空"
                  onConfirm={() => void clearOne("all", "全部记忆")}
                />
              }
            />
          </KvListContent>
        </KvList>
      </SettingsSection>

      <FormDialog
        open={open}
        onOpenChange={setOpen}
        title={edit ? "编辑偏好" : "添加偏好"}
        submitLabel="保存"
        busyLabel="保存中…"
        busy={busy}
        disabled={!newKey.trim() || !newVal.trim()}
        onSubmit={() => void savePersona()}
      >
        <div className="flex flex-col gap-3 py-1">
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-[1fr_auto]">
            <Field label="键（如 language）">
              <Input
                value={newKey}
                disabled={!!edit}
                onChange={(e) => setNewKey(e.target.value)}
                placeholder="key"
                className="h-8 text-xs font-mono"
              />
            </Field>
            <Field label="范围">
              <SettingSelect
                value={newScope}
                onChange={(v) => setNewScope(v as Scope)}
                triggerClassName="w-full sm:w-44"
                ariaLabel="偏好范围"
                options={[
                  { value: "global", label: "全局" },
                  { value: "workspace", label: "仅当前工作区" },
                ]}
              />
            </Field>
          </div>
          <Field label="值（如 使用中文回复）">
            <Input
              value={newVal}
              onChange={(e) => setNewVal(e.target.value)}
              placeholder="value"
              className="h-8 text-xs"
            />
          </Field>
          {/* A scope change is a move, not a second entry — the row it came from
           *  is forgotten on save. Say so before it happens. */}
          {edit && edit.scope !== newScope && (
            <p className="rounded-lg bg-neutral-100 px-3 py-2 text-[11px] leading-relaxed text-neutral-600">
              保存会把这条偏好移到新范围，原范围不再保留
            </p>
          )}
        </div>
      </FormDialog>

      {/* Ask before the one part of memory that costs a wait — never a silent
       *  quarter-gigabyte download behind a switch flip. The fetch itself is
       *  reported in the row, so there is no second progress surface to fall
       *  out of step with it. */}
      <Dialog open={askSemantic} onOpenChange={(o) => setAskSemantic(o)}>
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle className="text-base">启用语义召回</DialogTitle>
            <DialogDescription>
              首次启用需要下载本地语义模型，约 241 MB。下载完成后完全离线运行，不会把对话内容发到任何地方。
            </DialogDescription>
          </DialogHeader>
          <DialogFooter className="gap-2">
            <Button variant="outline" size="sm" onClick={() => setAskSemantic(false)}>
              取消
            </Button>
            <Button
              size="sm"
              onClick={() => {
                setAskSemantic(false);
                enableSemantic();
              }}
            >
              下载并启用
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
