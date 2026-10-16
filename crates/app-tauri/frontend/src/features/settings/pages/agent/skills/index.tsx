// 技能 pane — pi skill dirs; only the two writable roots are editable.

import { SkeletonKvRows } from "@/components/ui/skeleton";
import { toast } from "sonner";
import { useMemo, useState } from "react";
import { Lock, Plus, Sparkles, SquarePen } from "@keyline-icons/react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { KvList, KvListContent } from "@/components/ui/kv-list";
import { TooltipSimple } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { SettingSelect } from "@/features/settings/components/index";
import { DeleteButton } from "@/features/settings/components/delete-button";
import { FormDialog } from "@/features/settings/components/form-dialog";
import {
  createSkill,
  readSkill,
  deleteSkill,
  type AgentOverview,
  type SkillItem,
} from "@/lib/agent-ipc/sessions";
import {
  EmptyState,
  Field,
  ItemIcon,
  ItemRow,
  PaneToolbar,
  ScopeTabs,
  SearchBox,
  Tag,
  matchQuery,
} from "@/features/settings/pages/agent/shared/index";

type Scope = "all" | "workspace" | "global";

/** Mirrors the backend's `valid_slug` — lowercase alnum plus `-`/`_`. */
const SLUG_RE = /^[a-z0-9_-]+$/;

/* ============================ 技能 ============================ */

export function SkillsPane({ ov, reload }: { ov: AgentOverview | null; reload: () => void }) {
  const [scope, setScope] = useState<Scope>("all");
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [desc, setDesc] = useState("");
  const [target, setTarget] = useState<"workspace" | "global">("workspace");
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  /** Editing an existing skill (createSkill overwrites its SKILL.md). */
  const [editName, setEditName] = useState<string | null>(null);

  const skills = ov?.skills ?? [];
  const counts: Record<Scope, number> = {
    all: skills.length,
    workspace: skills.filter((s) => !s.global).length,
    global: skills.filter((s) => s.global).length,
  };

  const filtered = useMemo(
    () =>
      skills.filter(
        (s) =>
          (scope === "all" || (scope === "global" ? s.global : !s.global)) &&
          matchQuery(query, s.name, s.description),
      ),
    [skills, scope, query],
  );

  const slugOk = !name.trim() || !!editName || SLUG_RE.test(name.trim());

  const startCreate = () => {
    setEditName(null);
    setName(""); setDesc(""); setBody("");
    setTarget(scope === "global" ? "global" : "workspace");
    setOpen(true);
  };

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
    if (!name.trim() || !slugOk) return;
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
      toast.error(editName ? "保存失败" : "创建失败", { description: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const filtering = !!query.trim() || scope !== "all";

  return (
    <div className="flex flex-col gap-3">
      <PaneToolbar>
        <ScopeTabs
          value={scope}
          onChange={setScope}
          options={[
            { value: "all", label: "全部", count: ov ? counts.all : undefined },
            { value: "workspace", label: "工作区", count: ov ? counts.workspace : undefined },
            { value: "global", label: "全局", count: ov ? counts.global : undefined },
          ]}
        />
        <SearchBox value={query} onChange={setQuery} />
        <Button size="sm" variant="outline" className="h-7 shrink-0 text-xs gap-1" onClick={startCreate}>
          <Plus className="h-3.5 w-3.5" /> 添加
        </Button>
      </PaneToolbar>

      <KvList>
        <KvListContent>
          {!ov ? (
            <SkeletonKvRows rows={3} />
          ) : filtered.length ? (
            filtered.map((s) => (
              <SkillRow
                key={`${s.global ? "g" : "w"}:${s.name}`}
                skill={s}
                onEdit={() => void startEditSkill(s.name)}
                onDelete={() => void doDeleteSkill(s.name)}
              />
            ))
          ) : (
            <EmptyState
              icon={<Sparkles className="h-4 w-4" />}
              title={
                query.trim()
                  ? "没有匹配的技能"
                  : scope === "workspace"
                    ? "工作区暂无技能"
                    : scope === "global"
                      ? "全局暂无技能"
                      : "暂无技能"
              }
              description={filtering ? "调整搜索词或切换范围查看" : "技能是按需加载的指令集，可在此创建第一个"}
              action={
                !filtering && (
                  <Button size="sm" variant="outline" className="h-7 text-xs gap-1" onClick={startCreate}>
                    <Plus className="h-3.5 w-3.5" /> 添加技能
                  </Button>
                )
              }
            />
          )}
        </KvListContent>
      </KvList>

      <FormDialog
        open={open}
        onOpenChange={setOpen}
        width="lg"
        title={editName ? "编辑技能" : "添加技能"}
        submitLabel={editName ? "保存" : "创建"}
        busyLabel={editName ? "保存中…" : "创建中…"}
        busy={busy}
        disabled={!name.trim() || !slugOk}
        onSubmit={() => void submit()}
      >
        <div className="flex flex-col gap-3 py-1">
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-[1fr_auto]">
            <Field label="名称（小写 slug）">
              <Input
                value={name}
                disabled={!!editName}
                onChange={(e) => setName(e.target.value)}
                placeholder="my-skill"
                aria-invalid={!slugOk}
                className={cn("h-8 text-xs font-mono", !slugOk && "border-red-400 focus-visible:border-red-400")}
              />
              {!slugOk && (
                <span className="text-[11px] text-red-600">仅限小写字母、数字、连字符与下划线</span>
              )}
            </Field>
            <Field label="位置">
              <SettingSelect
                value={target}
                onChange={(v) => setTarget(v as "workspace" | "global")}
                triggerClassName="w-full sm:w-44"
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
              placeholder="描述适用场景"
              className="h-8 text-xs"
            />
          </Field>
          <Field label="内容（SKILL.md 正文）">
            <Textarea
              value={body}
              onChange={(e) => setBody(e.target.value)}
              rows={12}
              placeholder="# 技能说明与步骤…"
              className="max-h-[50vh] min-h-[160px] resize-y font-mono text-[12px] leading-relaxed"
            />
          </Field>
        </div>
      </FormDialog>
    </div>
  );
}

/* ============================ 行 ============================ */

function SkillRow({
  skill: s,
  onEdit,
  onDelete,
}: {
  skill: SkillItem;
  onEdit: () => void;
  onDelete: () => void;
}) {
  return (
    <ItemRow
      icon={
        <ItemIcon>
          <Sparkles className="h-4 w-4" />
        </ItemIcon>
      }
      title={s.name}
      monoTitle
      tags={
        <>
          <Tag>{s.global ? "全局" : "工作区"}</Tag>
          {!s.writable && (
            <TooltipSimple content="已安装的技能包，只读">
              <span className="inline-flex shrink-0 text-neutral-400">
                <Lock className="h-3 w-3" />
              </span>
            </TooltipSimple>
          )}
        </>
      }
      description={s.description}
      emptyDescription="未填写描述"
      meta={s.path}
      actions={
        s.writable && (
          <>
            <Button variant="ghost" size="icon" className="h-6 w-6" title="编辑" onClick={onEdit}>
              <SquarePen className="h-3.5 w-3.5" />
            </Button>
            <DeleteButton
              confirmTitle="删除技能"
              confirmDescription={`确定要删除技能「${s.name}」吗？技能文件将被删除，此操作无法撤销。`}
              onConfirm={onDelete}
            />
          </>
        )
      }
    />
  );
}
