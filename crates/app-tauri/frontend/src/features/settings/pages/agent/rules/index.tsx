// 规则 pane — 始终生效 rules ride the system prompt; 按文件匹配 ones are read
// before the model edits a matching file.

import { SkeletonKvRows } from "@/components/ui/skeleton";
import { toast } from "sonner";
import { useMemo, useState } from "react";
import { ListCheck, Plus, SquarePen } from "@keyline-icons/react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { KvList, KvListContent } from "@/components/ui/kv-list";
import { cn } from "@/lib/utils";
import { SettingSelect, type SettingSelectOption } from "@/features/settings/components/index";
import { DeleteButton } from "@/features/settings/components/delete-button";
import { FormDialog } from "@/features/settings/components/form-dialog";
import {
  deleteRule,
  readRule,
  saveRule,
  type AgentOverview,
  type RuleItem,
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
type RuleScope = "workspace" | "global";
/** 生效方式 — the manifest's `always:` flag, as the dialog's two choices. */
type Mode = "always" | "globs";

/** Mirrors the backend's `valid_slug` — lowercase alnum plus `-`/`_`. */
const SLUG_RE = /^[a-z0-9_-]+$/;

const MODE_OPTIONS: SettingSelectOption[] = [
  { value: "always", label: "始终生效" },
  { value: "globs", label: "按文件匹配" },
];

/** The form keeps globs in one comma-separated input; newlines count too, so
 *  a pasted list parses the same way. */
function parseGlobs(text: string): string[] {
  return text
    .split(/[,\n]/)
    .map((g) => g.trim())
    .filter(Boolean);
}

/** First two globs as tags, the rest folded into 「+N」 — a rule with ten
 *  patterns must not push the row's actions off screen. */
function GlobTags({ globs }: { globs: string[] }) {
  if (!globs.length) return null;
  return (
    <>
      {globs.slice(0, 2).map((g) => (
        <Tag key={g} mono className="max-w-40">
          <span className="truncate">{g}</span>
        </Tag>
      ))}
      {globs.length > 2 && <Tag mono>+{globs.length - 2}</Tag>}
    </>
  );
}

/* ============================ 规则 ============================ */

export function RulesPane({ ov, reload }: { ov: AgentOverview | null; reload: () => void }) {
  const [scope, setScope] = useState<Scope>("all");
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [desc, setDesc] = useState("");
  const [target, setTarget] = useState<RuleScope>("workspace");
  const [mode, setMode] = useState<Mode>("always");
  const [globs, setGlobs] = useState("");
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  /** Editing an existing rule — its name is the file stem, so it stays locked. */
  const [editName, setEditName] = useState<string | null>(null);

  const rules = ov?.rules ?? [];
  const counts: Record<Scope, number> = {
    all: rules.length,
    workspace: rules.filter((r) => !r.global).length,
    global: rules.filter((r) => r.global).length,
  };
  const filtered = useMemo(
    () =>
      rules.filter(
        (r) =>
          (scope === "all" || (scope === "global" ? r.global : !r.global)) &&
          matchQuery(query, r.name, r.description),
      ),
    [rules, scope, query],
  );

  const parsed = parseGlobs(globs);
  const slugOk = !name.trim() || !!editName || SLUG_RE.test(name.trim());
  // A glob-matched rule with no pattern would never fire.
  const globsOk = mode === "always" || parsed.length > 0;
  const valid = !!name.trim() && slugOk && globsOk && !!body.trim();

  const startCreate = () => {
    setEditName(null);
    setName(""); setDesc(""); setGlobs(""); setBody("");
    setMode("always");
    setTarget(scope === "global" ? "global" : "workspace");
    setOpen(true);
  };

  const startEditRule = async (ruleName: string) => {
    try {
      const detail = await readRule(ruleName);
      setEditName(ruleName);
      setName(ruleName);
      setDesc(detail.description);
      setGlobs(detail.globs.join(", "));
      setBody(detail.body);
      setMode(detail.always ? "always" : "globs");
      setTarget(detail.scope);
      setOpen(true);
    } catch (e) {
      toast.error("读取失败", { description: String(e) });
    }
  };

  const doDeleteRule = async (ruleName: string, ruleScope: RuleScope) => {
    try {
      await deleteRule(ruleName, ruleScope);
      toast.success("已删除规则");
      reload();
    } catch (e) {
      toast.error("删除失败", { description: String(e) });
    }
  };

  const submit = async () => {
    if (!valid) return;
    setBusy(true);
    try {
      await saveRule({
        name: name.trim(),
        scope: target,
        description: desc.trim(),
        globs: parsed,
        always: mode === "always",
        body: body.trim(),
      });
      toast.success(editName ? "已更新规则" : "已创建规则");
      setOpen(false);
      setEditName(null);
      setName(""); setDesc(""); setGlobs(""); setBody("");
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
            { value: "global", label: "全局", count: ov ? counts.global : undefined },
            { value: "workspace", label: "工作区", count: ov ? counts.workspace : undefined },
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
            filtered.map((r) => (
              <RuleRow
                key={`${r.global ? "global" : "workspace"}:${r.name}`}
                rule={r}
                onEdit={() => void startEditRule(r.name)}
                onDelete={() => void doDeleteRule(r.name, r.global ? "global" : "workspace")}
              />
            ))
          ) : (
            <EmptyState
              icon={<ListCheck className="h-4 w-4" />}
              title={
                query.trim()
                  ? "没有匹配的规则"
                  : scope === "workspace"
                    ? "工作区暂无规则"
                    : scope === "global"
                      ? "全局暂无规则"
                      : "暂无规则"
              }
              description={
                filtering
                  ? "调整搜索词或切换范围查看"
                  : "规则是持续生效的细粒度约束，区别于按需加载的技能"
              }
              action={
                !filtering && (
                  <Button size="sm" variant="outline" className="h-7 text-xs gap-1" onClick={startCreate}>
                    <Plus className="h-3.5 w-3.5" /> 添加规则
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
        title={editName ? "编辑规则" : "添加规则"}
        submitLabel={editName ? "保存" : "创建"}
        busyLabel={editName ? "保存中…" : "创建中…"}
        busy={busy}
        disabled={!valid}
        onSubmit={() => void submit()}
      >
        <div className="flex flex-col gap-3 py-1">
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-[1fr_auto]">
            <Field label="名称（小写 slug）">
              <Input
                value={name}
                disabled={!!editName}
                onChange={(e) => setName(e.target.value)}
                placeholder="api-conventions"
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
                onChange={(v) => setTarget(v as RuleScope)}
                triggerClassName="w-full sm:w-44"
                options={[
                  { value: "workspace", label: "工作区" },
                  { value: "global", label: "全局" },
                ]}
                ariaLabel="规则位置"
              />
            </Field>
          </div>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-[1fr_auto]">
            <Field label="描述">
              <Input
                value={desc}
                onChange={(e) => setDesc(e.target.value)}
                placeholder="这条规则约束什么"
                className="h-8 text-xs"
              />
            </Field>
            <Field label="生效方式">
              <SettingSelect
                value={mode}
                onChange={(v) => setMode(v as Mode)}
                triggerClassName="w-full sm:w-44"
                options={MODE_OPTIONS}
                ariaLabel="规则生效方式"
              />
            </Field>
          </div>
          {mode === "globs" && (
            <Field label="文件匹配（逗号分隔）">
              <Input
                value={globs}
                onChange={(e) => setGlobs(e.target.value)}
                placeholder="src/**/*.ts, docs/*.md"
                aria-invalid={!globsOk}
                className={cn("h-8 text-xs font-mono", !globsOk && "border-red-400 focus-visible:border-red-400")}
              />
              {!globsOk && <span className="text-[11px] text-red-600">至少写一个匹配模式</span>}
            </Field>
          )}
          <Field label="规则内容">
            <Textarea
              value={body}
              onChange={(e) => setBody(e.target.value)}
              rows={10}
              placeholder="新增接口必须同时更新 OpenAPI 描述…"
              className="max-h-[50vh] min-h-[140px] font-mono text-[12px] leading-relaxed"
            />
          </Field>
          <p className="text-[11px] leading-relaxed text-neutral-400">
            规则在新会话或重新打开会话时生效
          </p>
        </div>
      </FormDialog>
    </div>
  );
}

/* ============================ 行 ============================ */

function RuleRow({
  rule: r,
  onEdit,
  onDelete,
}: {
  rule: RuleItem;
  onEdit: () => void;
  onDelete: () => void;
}) {
  return (
    <ItemRow
      icon={
        <ItemIcon>
          <ListCheck className="h-4 w-4" />
        </ItemIcon>
      }
      title={r.name}
      monoTitle
      tags={
        <>
          <Tag>{r.global ? "全局" : "工作区"}</Tag>
          {r.always ? <Tag tone="success">始终生效</Tag> : <Tag>按文件匹配</Tag>}
          {!r.always && <GlobTags globs={r.globs} />}
        </>
      }
      description={r.description}
      emptyDescription="未填写描述"
      meta={r.path}
      actions={
        <>
          <Button variant="ghost" size="icon" className="h-6 w-6" title="编辑" onClick={onEdit}>
            <SquarePen className="h-3.5 w-3.5" />
          </Button>
          <DeleteButton
            confirmTitle="删除规则"
            confirmDescription={`确定要删除规则「${r.name}」吗？其定义文件将被删除，此操作无法撤销。`}
            onConfirm={onDelete}
          />
        </>
      }
    />
  );
}
