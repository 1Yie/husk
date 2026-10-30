import { Button } from "@/components/ui/button";
import { refreshWorkspaceChanges, type WorkspaceChanges } from "./workspace-changes";

/** Stays visible even when there are no file rows (and hence no 改动 tab). */
export function WorkspaceChangesNotice({
  root,
  state,
}: {
  root: string;
  state?: WorkspaceChanges;
}) {
  if (!root || !state?.error) return null;
  return (
    <div role="alert" className="flex flex-none items-start gap-2 px-3 py-2 text-xs text-text-secondary">
      <div className="min-w-0 flex-1 max-h-32 overflow-y-auto break-words">
        <p className="font-medium text-text-primary">工作区改动读取失败</p>
        <p className="mt-1">
          {state.at > 0
            ? "显示上次成功结果，可能已过期。"
            : "尚未成功读取，无法确认是否有未提交改动。"}
        </p>
        <p className="mt-1 whitespace-pre-wrap select-text">{state.error}</p>
      </div>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="flex-none"
        aria-label="重新读取工作区改动"
        disabled={state.loading}
        onClick={() => void refreshWorkspaceChanges(root)}
      >
        {state.loading ? "重试中…" : "重试"}
      </Button>
    </div>
  );
}
