// 「记忆」pane — the read/write side of the three memory write paths
// (remember tool, /remember command, turn-end distillation). Persona is
// global key/value; facts + episodes are scoped to this workspace.

import { SkeletonKvRows } from "@/components/ui/skeleton";
import { toast } from "sonner";
import { useEffect, useState } from "react";
import {
  Bin,
  Brain,
  CircleAlert,
  CircleUser,
  Clock,
  Database,
  Plus,
  Sparkles,
  SquarePen,
} from "@keyline-icons/react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { KvList, KvListContent } from "@/components/ui/kv-list";
import { SettingsSection } from "@/features/settings/components";
import { ArmedDeleteButton } from "@/features/settings/components/armed-delete";
import { FormDialog } from "@/features/settings/components/form-dialog";
import {
  EmptyState,
  Field,
  ItemIcon,
  ItemRow,
  SearchBox,
  Tag,
  matchQuery,
} from "@/features/settings/pages/agent/shared/index";
import { timeAgo } from "@/lib/time";
import { cn } from "@/lib/utils";
import {
  clearMemory,
  forgetMemory,
  getMemoryOverview,
  removeMemoryFact,
  setDefaultPrefs,
  setMemoryPersona,
  type MemoryOverview,
} from "@/lib/agent-ipc/sessions";

/** Confidence → tag tone: the distiller's certainty at a glance. */
const confidenceTone = (c: number) => (c >= 0.8 ? "success" : c >= 0.5 ? "neutral" : "warn");

export function MemoryPane() {
  const [ov, setOv] = useState<MemoryOverview | null>(null);
  const [open, setOpen] = useState(false);
  /** Persona key being edited — saving overwrites it, so the key stays locked. */
  const [editKey, setEditKey] = useState<string | null>(null);
  const [newKey, setNewKey] = useState("");
  const [newVal, setNewVal] = useState("");
  const [busy, setBusy] = useState(false);
  const [factQuery, setFactQuery] = useState("");

  const reload = () => {
    void getMemoryOverview()
      .then(setOv)
      .catch(() => {});
  };
  useEffect(reload, []);

  const savePref = (patch: Parameters<typeof setDefaultPrefs>[0]) =>
    setDefaultPrefs(patch).catch((e) => toast.error("保存失败", { description: String(e) }));

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
    setEditKey(null);
    setNewKey("");
    setNewVal("");
    setOpen(true);
  };

  const startEdit = (key: string, value: string) => {
    setEditKey(key);
    setNewKey(key);
    setNewVal(value);
    setOpen(true);
  };

  const savePersona = async () => {
    const k = newKey.trim();
    const v = newVal.trim();
    if (!k || !v) return;
    setBusy(true);
    try {
      await setMemoryPersona(k, v);
      toast.success(editKey ? "已更新偏好" : "已记住偏好");
      setOpen(false);
      setEditKey(null);
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
  const shownFacts = facts.filter((f) => matchQuery(factQuery, f.text));
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
                    if (ov) setOv({ ...ov, enabled: v });
                    void savePref({ memory_enabled: v });
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
                    if (ov) setOv({ ...ov, distill: v });
                    void savePref({ memory_distill: v });
                  }}
                />
              }
            />
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
        description="全局生效，每轮对话都会附带"
        actions={
          <>
            {count(ov ? persona.length : undefined)}
            <Button size="sm" variant="outline" className="h-7 text-xs gap-1" onClick={startAdd}>
              <Plus className="h-3.5 w-3.5" /> 添加
            </Button>
          </>
        }
      >
        <KvList className={cn(inactive && "opacity-60")}>
          <KvListContent>
            {!ov ? (
              <SkeletonKvRows rows={2} />
            ) : persona.length ? (
              persona.map((p) => (
                <ItemRow
                  key={p.key}
                  icon={
                    <ItemIcon>
                      <CircleUser className="h-4 w-4" />
                    </ItemIcon>
                  }
                  title={p.key}
                  monoTitle
                  description={p.value}
                  actions={
                    <>
                      <Button variant="ghost" size="icon" className="h-6 w-6" title="编辑"
                        onClick={() => startEdit(p.key, p.value)}>
                        <SquarePen className="h-3.5 w-3.5" />
                      </Button>
                      <ArmedDeleteButton
                        onConfirm={() =>
                          void forgetMemory(p.key)
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
                title="暂无偏好"
                description="对话中说「记住…」或在此手动添加，例如回复语言、代码风格"
                action={
                  <Button size="sm" variant="outline" className="h-7 text-xs gap-1" onClick={startAdd}>
                    <Plus className="h-3.5 w-3.5" /> 添加偏好
                  </Button>
                }
              />
            )}
          </KvListContent>
        </KvList>
      </SettingsSection>

      {/* ---------- 记忆事实（当前工作区） ---------- */}
      <SettingsSection
        title="记忆事实"
        description="当前工作区，由蒸馏或 remember 工具写入"
        actions={count(ov ? facts.length : undefined)}
      >
        {facts.length > 5 && (
          <div className="flex">
            <SearchBox value={factQuery} onChange={setFactQuery} placeholder="搜索事实" />
          </div>
        )}
        <KvList className={cn(inactive && "opacity-60")}>
          <KvListContent>
            {!ov ? (
              <SkeletonKvRows rows={3} />
            ) : shownFacts.length ? (
              shownFacts.map((f) => (
                <ItemRow
                  key={f.id}
                  icon={
                    <ItemIcon>
                      <Database className="h-4 w-4" />
                    </ItemIcon>
                  }
                  title={f.text}
                  wrapTitle
                  tags={
                    <Tag tone={confidenceTone(f.confidence)} className="mt-0.5 tabular-nums">
                      {Math.round(f.confidence * 100)}%
                    </Tag>
                  }
                  description={f.created_at ? `记录于 ${timeAgo(f.created_at)}` : undefined}
                  actions={
                    <ArmedDeleteButton
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
                <ArmedDeleteButton
                  variant="text"
                  label="清空"
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
                <ArmedDeleteButton
                  variant="text"
                  label="清空"
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
                <ArmedDeleteButton
                  variant="text"
                  label="全部清空"
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
        title={editKey ? "编辑偏好" : "添加偏好"}
        submitLabel="保存"
        busyLabel="保存中…"
        busy={busy}
        disabled={!newKey.trim() || !newVal.trim()}
        onSubmit={() => void savePersona()}
      >
        <div className="flex flex-col gap-3 py-1">
          <Field label="键（如 language）">
            <Input
              value={newKey}
              disabled={!!editKey}
              onChange={(e) => setNewKey(e.target.value)}
              placeholder="key"
              className="h-8 text-xs font-mono"
            />
          </Field>
          <Field label="值（如 使用中文回复）">
            <Input
              value={newVal}
              onChange={(e) => setNewVal(e.target.value)}
              placeholder="value"
              className="h-8 text-xs"
            />
          </Field>
        </div>
      </FormDialog>
    </div>
  );
}
