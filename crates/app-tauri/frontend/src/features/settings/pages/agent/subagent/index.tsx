// 子代理 pane — builtin specs plus `*.md` manifests.

import { SkeletonKvRows } from "@/components/ui/skeleton";
import { toast } from "sonner";
import { useMemo, useState } from "react";
import { Bot, Plus, SquarePen } from "@keyline-icons/react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { KvList, KvListContent } from "@/components/ui/kv-list";
import { SettingSelect } from "@/features/settings/components/index";
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

/** Limits a spec carries, as compact tag text. */
function limits(a: SubagentItem): string[] {
  return [
    a.max_tool_rounds != null ? `${a.max_tool_rounds} 轮` : null,
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
  const [busy, setBusy] = useState(false);
  /** Editing an existing manifest (createSubagent overwrites it); builtins have
   *  no file, so they stay read-only. */
  const [editName, setEditName] = useState<string | null>(null);

  const agents = ov?.subagents ?? [];
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
    setName(""); setDesc(""); setPrompt("");
    setScope("workspace");
    setOpen(true);
  };

  const startEditSubagent = async (agentName: string) => {
    try {
      const detail = await readSubagent(agentName);
      setEditName(agentName);
      setName(agentName);
      setDesc(detail.description);
      setPrompt(detail.prompt);
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
      });
      toast.success(editName ? "已更新子代理" : "已创建子代理");
      setOpen(false);
      setEditName(null);
      setName(""); setDesc(""); setPrompt("");
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
                    {limits(a).map((t) => (
                      <Tag key={t} className="tabular-nums">{t}</Tag>
                    ))}
                  </>
                }
                description={a.description}
                emptyDescription="未填写描述"
                meta={a.path}
                actions={
                  !a.builtin && (
                    <>
                      <Button variant="ghost" size="icon" className="h-6 w-6" title="编辑"
                        onClick={() => void startEditSubagent(a.name)}>
                        <SquarePen className="h-3.5 w-3.5" />
                      </Button>
                      <DeleteButton
                        confirmTitle="删除子代理"
                        confirmDescription={`确定要删除子代理「${a.name}」吗？其定义文件将被删除，此操作无法撤销。`}
                        onConfirm={() => void doDeleteSubagent(a.name)}
                      />
                    </>
                  )
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
        title={editName ? "编辑子代理" : "添加子代理"}
        submitLabel={editName ? "保存" : "创建"}
        busyLabel={editName ? "保存中…" : "创建中…"}
        busy={busy}
        disabled={!name.trim() || !prompt.trim()}
        onSubmit={() => void submit()}
      >
          <div className="flex flex-col gap-3 py-1">
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
            <Field label="描述">
              <Input
                value={desc}
                onChange={(e) => setDesc(e.target.value)}
                placeholder="这个子代理擅长什么"
                className="h-8 text-xs"
              />
            </Field>
            <Field label="系统提示词（子代理的行为指令）">
              <Textarea
                value={prompt}
                onChange={(e) => setPrompt(e.target.value)}
                rows={12}
                placeholder="你是一个代码审查专家，专注于…"
                className="max-h-[50vh] min-h-[160px] resize-y font-mono text-[12px] leading-relaxed"
              />
            </Field>
          </div>
      </FormDialog>
    </div>
  );
}
