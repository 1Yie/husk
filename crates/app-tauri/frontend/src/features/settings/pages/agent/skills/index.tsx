// 技能 pane — pi skill dirs; only the two writable roots are editable.

import { toast } from "sonner";
import { useState } from "react";
import {
  Plus,
  Sparkles,
  SquarePen,
} from "@keyline-icons/react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { KvList, KvListContent, KvRow } from "@/components/ui/kv-list";
import { cn } from "@/lib/utils";
import { SettingSelect } from "@/features/settings/components/index";
import { ArmedDeleteButton } from "@/features/settings/components/armed-delete";
import { FormDialog } from "@/features/settings/components/form-dialog";
import { createSkill, readSkill, deleteSkill, type AgentOverview } from "@/lib/agent-ipc/sessions";
import { Field } from "@/features/settings/pages/agent/shared/index";

/* ============================ 技能 ============================ */

export function SkillsPane({ ov, reload }: { ov: AgentOverview | null; reload: () => void }) {
  const [scope, setScope] = useState<"all" | "workspace" | "global">("all");
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [desc, setDesc] = useState("");
  const [target, setTarget] = useState<"workspace" | "global">("workspace");
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  /** Editing an existing skill (createSkill overwrites its SKILL.md). */
  const [editName, setEditName] = useState<string | null>(null);
  /** Skill whose delete button was armed — the second click confirms. */

  const filtered = (ov?.skills ?? []).filter(
    (s) => scope === "all" || (scope === "global" ? s.global : !s.global),
  );

  const startEditSkill = async (skillName: string) => {
    try {
      const detail = await readSkill(skillName);
      setEditName(skillName);
      setName(skillName);
      setDesc(detail.description);
      setBody(detail.body);
      setTarget(detail.scope);
      setOpen(true);
    } catch (e) {
      toast.error("读取失败", { description: String(e) });
    }
  };

  const doDeleteSkill = async (skillName: string) => {
    try {
      await deleteSkill(skillName);
      toast.success("已删除技能");
      reload();
    } catch (e) {
      toast.error("删除失败", { description: String(e) });
    }
  };

  const submit = async () => {
    if (!name.trim()) return;
    setBusy(true);
    try {
      await createSkill({
        name: name.trim(),
        description: desc.trim(),
        scope: target,
        body: body.trim() || `# ${name.trim()}\n`,
      });
      toast.success(editName ? "已更新技能" : "已创建技能");
      setOpen(false);
      setEditName(null);
      setName(""); setDesc(""); setBody("");
      reload();
    } catch (e) {
      toast.error("创建失败", { description: String(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-1">
          {(["all", "workspace", "global"] as const).map((v) => (
            <button
              key={v}
              onClick={() => setScope(v)}
              className={cn(
                "h-7 px-2.5 rounded-md text-xs transition-colors cursor-pointer",
                scope === v
                  ? "bg-[color-mix(in_srgb,var(--husk-black)_6%,transparent)] text-neutral-900"
                  : "text-neutral-500 hover:text-neutral-900",
              )}
            >
              {v === "all" ? "全部" : v === "workspace" ? "工作区" : "全局"}
            </button>
          ))}
        </div>
        <Button
          size="sm"
          variant="outline"
          className="h-7 text-xs gap-1"
          onClick={() => {
            setEditName(null);
            setName(""); setDesc(""); setBody("");
            setOpen(true);
          }}
        >
          <Plus className="h-3.5 w-3.5" /> 添加
        </Button>
      </div>
      <KvList>
        <KvListContent>
          {filtered.length ? (
            filtered.map((s) => (
              <KvRow
                key={s.path}
                label={s.name}
                description={
                  s.description ? (
                    <span className="line-clamp-3">{s.description}</span>
                  ) : undefined
                }
                icon={<Sparkles className="h-4 w-4" />}
              >
                <div className="flex items-center gap-2">
                  {s.global && (
                    <Badge variant="secondary" className="text-[10px] px-1.5 py-0 h-4">
                      全局
                    </Badge>
                  )}
                  <span className="text-[11px] text-neutral-500 font-mono max-w-[220px] truncate">
                    {s.path}
                  </span>
                  {s.writable && (
                    <>
                      <Button variant="ghost" size="icon" className="h-6 w-6" title="编辑"
                        onClick={() => void startEditSkill(s.name)}>
                        <SquarePen className="h-3.5 w-3.5" />
                      </Button>
                      <ArmedDeleteButton onConfirm={() => void doDeleteSkill(s.name)} />
                    </>
                  )}
                
                </div>
              </KvRow>
            ))
          ) : (
            <KvRow
              label={
                scope === "workspace"
                  ? "工作区暂无技能"
                  : scope === "global"
                    ? "全局暂无技能"
                    : "暂无技能"
              }
              description="点右上角「添加」创建一个，或切换其他范围查看"
            />
          )}
        </KvListContent>
      </KvList>

      <FormDialog
        open={open}
        onOpenChange={setOpen}
        title={editName ? "编辑技能" : "添加技能"}
        submitLabel={editName ? "保存" : "创建"}
        busyLabel="创建中…"
        busy={busy}
        disabled={!name.trim()}
        onSubmit={() => void submit()}
      >
          <div className="flex flex-col gap-3 py-1">
            <div className="grid grid-cols-2 gap-3">
              <Field label="名称（小写 slug）">
                <Input
                  value={name}
                  disabled={!!editName}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="my-skill"
                  className="h-8 text-xs font-mono"
                />
              </Field>
              <Field label="位置">
                <SettingSelect
                  value={target}
                  onChange={(v) => setTarget(v as "workspace" | "global")}
                  options={[
                    { value: "workspace", label: "工作区" },
                    { value: "global", label: "全局" },
                  ]}
                />
              </Field>
            </div>
            <Field label="描述">
              <Input
                value={desc}
                onChange={(e) => setDesc(e.target.value)}
                placeholder="什么时候用这个技能"
                className="h-8 text-xs"
              />
            </Field>
            <Field label="内容（SKILL.md 正文）">
              <Textarea
                value={body}
                onChange={(e) => setBody(e.target.value)}
                rows={6}
                placeholder="# 技能说明与步骤…"
                className="font-mono text-[12px] leading-relaxed"
              />
            </Field>
          </div>
      </FormDialog>
    </div>
  );
}
