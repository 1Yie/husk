// 指令 pane — AGENTS.md editors (global + workspace).
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { toast } from "sonner";
import { getInstructions, setInstructions, type InstructionsSet } from "@/lib/agent-ipc/sessions";

/* ============================ 指令 ============================ */

/** One instructions editor — defined at MODULE level, not inside the
 * pane: a component declared in the render body is a new type every
 * render, so React unmounted+remounted it (and its Textarea) on every
 * keystroke — which is what made the input drop focus while typing. */
function InstructionsBlock({
  scope,
  title,
  file,
  value,
  saving,
  onChange,
  onSave,
}: {
  scope: "global" | "workspace";
  title: string;
  file?: { path: string | null };
  value: string;
  saving: boolean;
  onChange: (v: string) => void;
  onSave: () => void;
}) {
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-baseline justify-between gap-2">
        <span className="text-sm font-semibold text-neutral-900">{title}</span>
        {file?.path && (
          <span className="text-[11px] text-neutral-500 font-mono truncate">
            {file.path}
          </span>
        )}
      </div>
      <Textarea
        value={value}
        onChange={(e) => onChange(e.target.value)}
        rows={10}
        className="font-mono text-[12px] leading-relaxed resize-y min-h-40 dark:bg-active"
        placeholder="在此编写自定义指令，会追加到每个新会话的系统提示词末尾"
      />
      <div className="flex justify-end">
        <Button
          size="sm"
          variant="outline"
          className="h-7 text-xs"
          disabled={saving}
          onClick={onSave}
        >
          {saving ? "保存中…" : "保存"}
        </Button>
      </div>
    </div>
  );
}

export function InstructionsPane() {
  const [data, setData] = useState<InstructionsSet | null>(null);
  const [drafts, setDrafts] = useState({ global: "", workspace: "" });
  const [saving, setSaving] = useState<string | null>(null);

  useEffect(() => {
    void getInstructions()
      .then((d) => {
        setData(d);
        setDrafts({ global: d.global.content, workspace: d.workspace.content });
      })
      .catch(() => {});
  }, []);

  const save = async (scope: "global" | "workspace") => {
    setSaving(scope);
    try {
      await setInstructions(scope, drafts[scope]);
      toast.success("已保存");
    } catch (e) {
      toast.error("保存失败", { description: String(e) });
    } finally {
      setSaving(null);
    }
  };

  return (
    <div className="flex flex-col gap-8">
      <InstructionsBlock
        scope="global"
        title="全局指令"
        file={data?.global}
        value={drafts.global}
        saving={saving === "global"}
        onChange={(v) => setDrafts((d) => ({ ...d, global: v }))}
        onSave={() => void save("global")}
      />
      <InstructionsBlock
        scope="workspace"
        title="项目指令"
        file={data?.workspace}
        value={drafts.workspace}
        saving={saving === "workspace"}
        onChange={(v) => setDrafts((d) => ({ ...d, workspace: v }))}
        onSave={() => void save("workspace")}
      />
    </div>
  );
}
