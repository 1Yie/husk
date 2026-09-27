// 「记忆」pane — the read/write side of the three memory write paths
// (remember tool, /remember command, turn-end distillation). Persona is
// global key/value; facts + episodes are scoped to this workspace.

import { toast } from "sonner";
import { useEffect, useState } from "react";
import { Brain, CircleUser, Clock, Database, Plus, Sparkles } from "@keyline-icons/react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Badge } from "@/components/ui/badge";
import { KvList, KvListContent, KvRow } from "@/components/ui/kv-list";
import { SettingsSection } from "@/features/settings/components";
import { ArmedDeleteButton } from "@/features/settings/components/armed-delete";
import { FormDialog } from "@/features/settings/components/form-dialog";
import { Field } from "@/features/settings/pages/agent/shared/index";
import {
  clearMemory,
  forgetMemory,
  getMemoryOverview,
  removeMemoryFact,
  setDefaultPrefs,
  setMemoryPersona,
  type MemoryOverview,
} from "@/lib/agent-ipc/sessions";

export function MemoryPane() {
  const [ov, setOv] = useState<MemoryOverview | null>(null);
  const [open, setOpen] = useState(false);
  const [newKey, setNewKey] = useState("");
  const [newVal, setNewVal] = useState("");
  const [busy, setBusy] = useState(false);

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

  const addPersona = async () => {
    const k = newKey.trim();
    const v = newVal.trim();
    if (!k || !v) return;
    setBusy(true);
    try {
      await setMemoryPersona(k, v);
      toast.success("已记住偏好");
      setOpen(false);
      setNewKey("");
      setNewVal("");
      reload();
    } catch (e) {
      toast.error("保存失败", { description: String(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col gap-8 w-full">
      <SettingsSection title="记忆开关">
        <KvList>
          <KvListContent>
            <KvRow
              label="启用记忆"
              icon={<Brain className="h-4 w-4" />}
            >
              <Switch
                checked={ov?.enabled ?? true}
                onCheckedChange={(v) => {
                  if (ov) setOv({ ...ov, enabled: v });
                  void savePref({ memory_enabled: v });
                }}
              />
            </KvRow>
            <KvRow
              label="模型蒸馏"
              description="每轮结束后由模型提取任务摘要、事实与偏好"
              icon={<Sparkles className="h-4 w-4" />}
            >
              <Switch
                checked={ov?.distill ?? true}
                onCheckedChange={(v) => {
                  if (ov) setOv({ ...ov, distill: v });
                  void savePref({ memory_distill: v });
                }}
              />
            </KvRow>
          </KvListContent>
        </KvList>
      </SettingsSection>

      {ov?.error && (
        <div className="rounded-xl border border-red-200 bg-red-50 px-4 py-3 text-xs text-red-700">
          {ov.error} —— 显示为空并非没有记忆，是库没能打开
        </div>
      )}

      <SettingsSection
        title="个人偏好"
        actions={
          <Button
            size="sm"
            variant="outline"
            className="h-7 text-xs gap-1"
            onClick={() => setOpen(true)}
          >
            <Plus className="h-3.5 w-3.5" /> 添加
          </Button>
        }
      >
        <KvList>
          <KvListContent>
            {(ov?.persona ?? []).length === 0 ? (
              <KvRow
                label="暂无偏好"
              />
            ) : (
              (ov?.persona ?? []).map((p) => (
              <KvRow key={p.key} label={p.key} description={p.value} icon={<CircleUser className="h-4 w-4" />}>
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
              </KvRow>
              ))
            )}
          </KvListContent>
        </KvList>
      </SettingsSection>

      {/* ---------- 记忆事实（当前工作区） ---------- */}
      <SettingsSection
        title="记忆事实"
        actions={
          <Badge variant="secondary" className="text-[10px] px-1.5 py-0 h-4">
            {ov?.facts.length ?? 0} 条
          </Badge>
        }
      >
        <KvList>
          <KvListContent>
            {(ov?.facts ?? []).length === 0 ? (
              <KvRow
                label="暂无事实"
              />
            ) : (
              (ov?.facts ?? []).map((f) => (
              <KvRow
                key={f.id}
                label={f.text}
                description={`置信度 ${Math.round(f.confidence * 100)}%`}
                icon={<Database className="h-4 w-4" />}
              >
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
              </KvRow>
              ))
            )}
          </KvListContent>
        </KvList>
      </SettingsSection>

      {/* ---------- 数据 ---------- */}
      <SettingsSection title="数据">
        <KvList>
          <KvListContent>
            <KvRow
              label="记忆事实"
              description={`当前工作区 ${ov?.facts.length ?? 0} 条`}
              icon={<Database className="h-4 w-4" />}
            >
              <ArmedDeleteButton
                variant="text"
                label="清空"
                onConfirm={() => void clearOne("facts", "记忆事实")}
              />
            </KvRow>
            <KvRow
              label="轮次经历"
              description={`当前工作区 ${ov?.episodes ?? 0} 条`}
              icon={<Clock className="h-4 w-4" />}
            >
              <ArmedDeleteButton
                variant="text"
                label="清空"
                onConfirm={() => void clearOne("episodes", "轮次经历")}
              />
            </KvRow>
            <KvRow
              label="全部记忆"
              description="偏好 + 事实 + 轮次经历，一次清空"
              icon={<Database className="h-4 w-4" />}
            >
              <ArmedDeleteButton
                variant="text"
                label="全部清空"
                onConfirm={() => void clearOne("all", "全部记忆")}
              />
            </KvRow>
          </KvListContent>
        </KvList>
      </SettingsSection>

      <FormDialog
        open={open}
        onOpenChange={setOpen}
        title="添加偏好"
        submitLabel="保存"
        busyLabel="保存中…"
        busy={busy}
        disabled={!newKey.trim() || !newVal.trim()}
        onSubmit={() => void addPersona()}
      >
        <div className="flex flex-col gap-3 py-1">
          <Field label="键（如 language）">
            <Input
              value={newKey}
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
