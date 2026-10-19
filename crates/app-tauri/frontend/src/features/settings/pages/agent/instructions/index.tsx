// AGENTS.md editors for global and project instructions.
import { useEffect, useId, useState } from "react";
import { FileText, Folder, Globe } from "@keyline-icons/react";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { KvList } from "@/components/ui/kv-list";
import { Textarea } from "@/components/ui/textarea";
import { EmptyState, ItemIcon, Tag } from "@/features/settings/components";
import { getInstructions, setInstructions, type InstructionsSet } from "@/lib/agent-ipc/sessions";
import { toast } from "sonner";

type Scope = "global" | "workspace";

// Keep this at module scope so typing never remounts the editor.
function InstructionsBlock({
  scope,
  file,
  value,
  saving,
  loading,
  dirty,
  onChange,
  onSave,
}: {
  scope: Scope;
  file?: { path: string | null };
  value: string;
  saving: boolean;
  loading: boolean;
  dirty: boolean;
  onChange: (v: string) => void;
  onSave: () => void;
}) {
  const editorId = useId();
  const global = scope === "global";
  const title = global ? "全局指令" : "项目指令";
  const Icon = global ? Globe : Folder;

  return (
    <KvList>
      <div className="flex items-start gap-3 px-5 py-4">
        <ItemIcon><Icon className="h-4 w-4" /></ItemIcon>
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <div className="flex flex-wrap items-center gap-2">
            <label htmlFor={editorId} className="text-sm font-medium text-neutral-900">{title}</label>
            <Tag mono>AGENTS.md</Tag>
          </div>
          <span className="text-xs text-neutral-500">{global ? "应用于所有项目" : "仅应用于当前项目"}</span>
          {file?.path && (
            <span title={file.path} className="max-w-full truncate font-mono text-[11px] text-neutral-500 select-text">
              {file.path}
            </span>
          )}
        </div>
      </div>
      <div className="px-5 pb-4">
        {loading ? (
          <Skeleton className="h-52 w-full animate-pulse rounded-xl" />
        ) : (
          <Textarea
            id={editorId}
            value={value}
            onChange={(e) => onChange(e.target.value)}
            onKeyDown={(e) => {
              if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
                e.preventDefault();
                if (dirty && !saving) onSave();
              }
            }}
            rows={9}
            shellClassName="shadow-none"
            className="min-h-44 max-h-[60vh] font-mono text-xs leading-6"
            placeholder={global ? "回复语言、代码风格等通用偏好…" : "项目约定、开发流程、测试命令…"}
            spellCheck={false}
          />
        )}
      </div>
      <div className="flex flex-wrap items-center justify-between gap-3 border-t border-hairline px-5 py-3">
        <div className="flex items-center gap-2 text-[11px] text-neutral-500" role="status">
          {dirty && <Tag>未保存</Tag>}
        </div>
        <Button
          size="sm"
          variant="outline"
          className="h-7 text-xs"
          aria-label={`保存${title}`}
          disabled={saving || loading || !dirty}
          onClick={onSave}
        >
          {saving ? "保存中…" : "保存"}
        </Button>
      </div>
    </KvList>
  );
}

export function InstructionsPane() {
  const [data, setData] = useState<InstructionsSet | null>(null);
  const [drafts, setDrafts] = useState({ global: "", workspace: "" });
  const [saving, setSaving] = useState({ global: false, workspace: false });
  const [loadError, setLoadError] = useState("");
  const [loadAttempt, setLoadAttempt] = useState(0);

  useEffect(() => {
    let alive = true;
    setLoadError("");
    void getInstructions()
      .then((d) => {
        if (!alive) return;
        setData(d);
        setDrafts({ global: d.global.content, workspace: d.workspace.content });
      })
      .catch((e) => { if (alive) setLoadError(String(e)); });
    return () => { alive = false; };
  }, [loadAttempt]);

  const save = async (scope: Scope) => {
    if (!data || saving[scope] || drafts[scope] === data[scope].content) return;
    // A user may keep typing during the request. Only mark this snapshot saved.
    const content = drafts[scope];
    setSaving((s) => ({ ...s, [scope]: true }));
    try {
      await setInstructions(scope, content);
      setData((d) => d ? { ...d, [scope]: { ...d[scope], content } } : d);
      toast.success("已保存");
    } catch (e) {
      toast.error("保存失败", { description: String(e) });
    } finally {
      setSaving((s) => ({ ...s, [scope]: false }));
    }
  };

  if (loadError) {
    return (
      <KvList>
        <EmptyState
          icon={<FileText className="h-4 w-4" />}
          title="无法读取指令"
          description={<span className="break-all">{loadError}</span>}
          action={<Button size="sm" variant="outline" onClick={() => setLoadAttempt((n) => n + 1)}>重试</Button>}
        />
      </KvList>
    );
  }

  return (
    <div className="flex min-w-0 flex-col gap-5">
      {(["global", "workspace"] as const).map((scope) => (
        <InstructionsBlock
          key={scope}
          scope={scope}
          file={data?.[scope]}
          value={drafts[scope]}
          saving={saving[scope]}
          loading={!data}
          dirty={!!data && drafts[scope] !== data[scope].content}
          onChange={(v) => setDrafts((d) => ({ ...d, [scope]: v }))}
          onSave={() => void save(scope)}
        />
      ))}
    </div>
  );
}
