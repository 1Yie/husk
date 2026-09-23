// 子代理 pane — builtin specs plus `*.md` manifests.

import { toast } from "sonner";
import { useState } from "react";
import {
  Bot,
  Plus,
  SquarePen,
} from "@keyline-icons/react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { KvList, KvListContent, KvRow } from "@/components/ui/kv-list";
import { SettingSelect } from "@/features/settings/components/index";
import { ArmedDeleteButton } from "@/features/settings/components/armed-delete";
import { FormDialog } from "@/features/settings/components/form-dialog";
import { readSubagent, deleteSubagent, createSubagent, type AgentOverview } from "@/lib/agent-ipc/sessions";
import { Field } from "@/features/settings/pages/agent/shared/index";

/* ============================ SubAgent ============================ */

export function SubagentPane({ ov, reload }: { ov: AgentOverview | null; reload: () => void }) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [desc, setDesc] = useState("");
  const [scope, setScope] = useState<"workspace" | "global">("workspace");
  const [prompt, setPrompt] = useState("");
  const [busy, setBusy] = useState(false);
  /** Editing an existing manifest (createSubagent overwrites it); builtins have
   *  no file, so they stay read-only. */
  const [editName, setEditName] = useState<string | null>(null);

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
      toast.error("创建失败", { description: String(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-end">
        <Button
          size="sm"
          variant="outline"
          className="h-7 text-xs gap-1"
          onClick={() => {
            setEditName(null);
            setName(""); setDesc(""); setPrompt("");
            setOpen(true);
          }}
        >
          <Plus className="h-3.5 w-3.5" /> 添加
        </Button>
      </div>
      <KvList>
        <KvListContent>
          {ov?.subagents.map((a) => (
            <KvRow
              key={a.name}
              label={a.name}
              description={a.description || undefined}
              icon={<Bot className="h-4 w-4" />}
            >
              <div className="flex items-center gap-2">
                {!a.builtin && (
                  <>
                    <Button variant="ghost" size="icon" className="h-6 w-6" title="编辑"
                      onClick={() => void startEditSubagent(a.name)}>
                      <SquarePen className="h-3.5 w-3.5" />
                    </Button>
                    <ArmedDeleteButton onConfirm={() => void doDeleteSubagent(a.name)} />
                  </>
                )}
                {a.builtin ? (
                  <Badge variant="secondary" className="text-[10px] px-1.5 py-0 h-4">
                    内置
                  </Badge>
                ) : (
                  <>
                    <Badge variant="secondary" className="text-[10px] px-1.5 py-0 h-4">
                      {a.global ? "全局" : "工作区"}
                    </Badge>
                    {a.path && (
                      <span className="text-[11px] text-neutral-500 font-mono max-w-[200px] truncate">
                        {a.path}
                      </span>
                    )}
                  </>
                )}

              </div>
            </KvRow>
          ))}
        </KvListContent>
      </KvList>

      <FormDialog
        open={open}
        onOpenChange={setOpen}
        title={editName ? "编辑子代理" : "添加子代理"}
        submitLabel={editName ? "保存" : "创建"}
        busyLabel="创建中…"
        busy={busy}
        disabled={!name.trim() || !prompt.trim()}
        onSubmit={() => void submit()}
      >
          <div className="flex flex-col gap-3 py-1">
            <div className="grid grid-cols-2 gap-3">
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
                rows={7}
                placeholder="你是一个代码审查专家，专注于…"
                className="font-mono text-[12px] leading-relaxed"
              />
            </Field>
          </div>
      </FormDialog>
    </div>
  );
}
