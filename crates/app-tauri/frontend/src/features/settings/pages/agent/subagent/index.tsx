// 子代理 pane — builtin specs plus `*.md` manifests.

import { SkeletonKvRows } from "@/components/ui/skeleton";
import { toast } from "sonner";
import { useMemo, useState } from "react";
import { Bot, Plus, SquarePen } from "@keyline-icons/react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { KvList, KvListContent } from "@/components/ui/kv-list";
import { SettingSelect, type SettingSelectOption } from "@/features/settings/components/index";
import { THINKING_LEVELS } from "@/lib/thinking";
import { DeleteButton } from "@/features/settings/components/delete-button";
import { FormDialog } from "@/features/settings/components/form-dialog";
import {
  readSubagent,
  deleteSubagent,
  createSubagent,
  type AgentOverview,
  type SubagentItem,
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

type Filter = "all" | "builtin" | "custom";

/** "" = no `thinking:` line: the child follows the subagent default, then the
 *  parent session's level. */
const THINKING_OPTIONS: SettingSelectOption[] = [
  { value: "", label: "跟随默认" },
  ...THINKING_LEVELS.map((l) => ({ value: l.value, label: l.label })),
];

/** Limits a spec carries, as compact tag text. `max_tool_rounds` bounds the
 * model/tool loop, so it caps **steps** (one model request each), not turns. */
function limits(a: SubagentItem): string[] {
  return [
    a.max_tool_rounds != null ? `${a.max_tool_rounds} 步` : null,
    a.timeout_secs != null
      ? a.timeout_secs >= 60 && a.timeout_secs % 60 === 0
        ? `${a.timeout_secs / 60} 分钟`
        : `${a.timeout_secs} 秒`
      : null,
  ].filter((x): x is string => !!x);
}

/* ============================ SubAgent ============================ */

export function SubagentPane({ ov, reload }: { ov: AgentOverview | null; reload: () => void }) {
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [desc, setDesc] = useState("");
  const [scope, setScope] = useState<"workspace" | "global">("workspace");
  const [prompt, setPrompt] = useState("");
  /** Manifest `model:` as `provider/model`; "" = follow the subagent default. */
  const [model, setModel] = useState("");
  const [thinking, setThinking] = useState("");
  const [busy, setBusy] = useState(false);
  /** Editing an existing manifest (createSubagent overwrites it). */
  const [editName, setEditName] = useState<string | null>(null);
  /** The name being edited has no file of its own — saving writes an override
   *  manifest, which shadows the built-in. */
  const [editBuiltin, setEditBuiltin] = useState(false);

  const agents = ov?.subagents ?? [];
  // Written qualified, so the manifest names the same model whichever
  // provider the parent session runs on.
  const modelOptions = useMemo(() => {
    const opts: SettingSelectOption[] = [{ value: "", label: "跟随子代理默认模型" }];
    for (const m of ov?.model.models ?? []) {
      opts.push({ value: `${m.provider}/${m.model}`, label: m.name || m.model, group: m.provider });
    }
    // A hand-written `model:` outside the list still shows as itself.
    if (model && !opts.some((o) => o.value === model)) {
      opts.push({ value: model, label: model, group: "自定义" });
    }
    return opts;
  }, [ov, model]);
  const counts: Record<Filter, number> = {
    all: agents.length,
    builtin: agents.filter((a) => a.builtin).length,
    custom: agents.filter((a) => !a.builtin).length,
  };
  const filtered = useMemo(
    () =>
      agents.filter(
        (a) =>
          (filter === "all" || (filter === "builtin" ? a.builtin : !a.builtin)) &&
          matchQuery(query, a.name, a.description),
      ),
    [agents, filter, query],
  );
  const startCreate = () => {
    setEditName(null);
    setEditBuiltin(false);
    setName(""); setDesc(""); setPrompt(""); setModel(""); setThinking("");
    setScope("workspace");
    setOpen(true);
  };

  const startEditSubagent = async (agentName: string) => {
    try {
      const detail = await readSubagent(agentName);
      setEditName(agentName);
      setEditBuiltin(detail.builtin === true);
      setName(agentName);
      setDesc(detail.description);
      setPrompt(detail.prompt);
      setModel(detail.model ?? "");
      setThinking(detail.thinking ?? "");
      setScope(detail.scope);
      setOpen(true);
    } catch (e) {
      toast.error("读取失败", { description: String(e) });
    }
  };

  const doDeleteSubagent = async (agentName: string) => {
    try {
      await deleteSubagent(agentName);
      toast.success("已删除子代理");
      reload();
    } catch (e) {
      toast.error("删除失败", { description: String(e) });
    }
  };

  const submit = async () => {
    if (!name.trim() || !prompt.trim()) return;
    setBusy(true);
    try {
      await createSubagent({
        name: name.trim(),
        description: desc.trim(),
        scope,
        prompt: prompt.trim(),
        model,
        thinking,
      });
      toast.success(
        editBuiltin ? "已生成内置子代理的覆盖配置" : editName ? "已更新子代理" : "已创建子代理",
      );
      setOpen(false);
      setEditName(null);
      setEditBuiltin(false);
      setName(""); setDesc(""); setPrompt(""); setModel(""); setThinking("");
      reload();
    } catch (e) {
      toast.error(editName ? "保存失败" : "创建失败", { description: String(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <PaneToolbar>
        <ScopeTabs
          value={filter}
          onChange={setFilter}
          options={[
            { value: "all", label: "全部", count: ov ? counts.all : undefined },
            { value: "builtin", label: "内置", count: ov ? counts.builtin : undefined },
            { value: "custom", label: "自定义", count: ov ? counts.custom : undefined },
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
            <SkeletonKvRows rows={2} />
          ) : filtered.length ? (
            filtered.map((a) => (
              <ItemRow
                key={`${a.builtin ? "b" : a.global ? "g" : "w"}:${a.name}`}
                icon={
                  <ItemIcon>
                    <Bot className="h-4 w-4" />
                  </ItemIcon>
                }
                title={a.name}
                monoTitle
                tags={
                  <>
                    <Tag>{a.builtin ? "内置" : a.global ? "全局" : "工作区"}</Tag>
                    {a.model && <Tag className="font-mono">{a.model}</Tag>}
                    {a.thinking && (
                      <Tag>
                        {THINKING_LEVELS.find((l) => l.value === a.thinking)?.label ?? a.thinking}
                      </Tag>
                    )}
                    {limits(a).map((t) => (
                      <Tag key={t} className="tabular-nums">{t}</Tag>
                    ))}
                  </>
                }
                description={a.description}
                emptyDescription="未填写描述"
                meta={a.path}
                actions={
                  <>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-6 w-6"
                      title={a.builtin ? "编辑（生成覆盖配置）" : "编辑"}
                      onClick={() => void startEditSubagent(a.name)}
                    >
                      <SquarePen className="h-3.5 w-3.5" />
                    </Button>
                    {!a.builtin && (
                      <DeleteButton
                        confirmTitle="删除子代理"
                        confirmDescription={`确定要删除子代理「${a.name}」吗？其定义文件将被删除，此操作无法撤销。`}
                        onConfirm={() => void doDeleteSubagent(a.name)}
                      />
                    )}
                  </>
                }
              />
            ))
          ) : (
            <EmptyState
              icon={<Bot className="h-4 w-4" />}
              title={
                query.trim()
                  ? "没有匹配的子代理"
                  : filter === "custom"
                    ? "暂无自定义子代理"
                    : "暂无子代理"
              }
              description={
                query.trim() || filter === "builtin"
                  ? "调整搜索词或切换范围查看"
                  : "子代理在独立上下文中执行委派任务，可在此定义专属角色"
              }
              action={
                !query.trim() && filter !== "builtin" && (
                  <Button size="sm" variant="outline" className="h-7 text-xs gap-1" onClick={startCreate}>
                    <Plus className="h-3.5 w-3.5" /> 添加子代理
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
        title={editName ? (editBuiltin ? "编辑内置子代理" : "编辑子代理") : "添加子代理"}
        submitLabel={editName ? "保存" : "创建"}
        busyLabel={editName ? "保存中…" : "创建中…"}
        busy={busy}
        disabled={!name.trim() || !prompt.trim()}
        onSubmit={() => void submit()}
      >
          <div className="flex flex-col gap-3 py-1">
            {editBuiltin && (
              <p className="rounded-lg bg-neutral-100 px-3 py-2 text-[11px] leading-relaxed text-neutral-600">
                内置子代理没有定义文件，保存会生成一份同名覆盖配置来替代它，工作区与全局各有一份时以工作区为准
              </p>
            )}
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-[1fr_auto]">
              <Field label="名称（小写 slug）">
                <Input
                  value={name}
                  disabled={!!editName}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="reviewer"
                  className="h-8 text-xs font-mono"
                />
              </Field>
              <Field label="位置">
                <SettingSelect
                  value={scope}
                  onChange={(v) => setScope(v as "workspace" | "global")}
                  triggerClassName="w-full sm:w-44"
                  options={[
                    { value: "workspace", label: "工作区" },
                    { value: "global", label: "全局" },
                  ]}
                />
              </Field>
            </div>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-[1fr_auto_auto]">
              <Field label="描述">
                <Input
                  value={desc}
                  onChange={(e) => setDesc(e.target.value)}
                  placeholder="这个子代理擅长什么"
                  className="h-8 text-xs"
                />
              </Field>
              <Field label="模型">
                <SettingSelect
                  value={model}
                  onChange={setModel}
                  triggerClassName="w-full sm:w-44"
                  options={modelOptions}
                  ariaLabel="子代理模型"
                />
              </Field>
              <Field label="思考等级">
                <SettingSelect
                  value={thinking}
                  onChange={setThinking}
                  triggerClassName="w-full sm:w-32"
                  options={THINKING_OPTIONS}
                  ariaLabel="子代理思考等级"
                />
              </Field>
            </div>
            <Field label="系统提示词（子代理的行为指令）">
              <Textarea
                value={prompt}
                onChange={(e) => setPrompt(e.target.value)}
                rows={12}
                placeholder="你是一个代码审查专家，专注于…"
                className="max-h-[50vh] min-h-[160px] font-mono text-[12px] leading-relaxed"
              />
            </Field>
          </div>
      </FormDialog>
    </div>
  );
}
