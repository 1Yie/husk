import { ArrowUpRight } from "@keyline-icons/react";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { MemoStreamdown } from "@/features/chat/components/chat-stream/markdown-stream";
import { openUrl } from "@/lib/agent-ipc/sessions";
import { useUpdateStore, type UpdatePhase } from "./update-store";

const PHASE_LABEL: Record<UpdatePhase, string> = {
  download: "正在下载",
  verify: "正在校验",
  install: "正在安装",
};

function mb(bytes: number) {
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export function UpdateDialog() {
  const state = useUpdateStore((s) => s.state);
  const open = useUpdateStore((s) => s.open);
  const setOpen = useUpdateStore((s) => s.setOpen);
  const onInstall = () => void useUpdateStore.getState().install();
  const onDismiss = () => useUpdateStore.getState().dismiss();
  const onIgnore = () => useUpdateStore.getState().ignore();
  const onRelaunch = () => void useUpdateStore.getState().relaunch();
  if (state.status === "idle") return null;
  const { info } = state;
  const installing = state.status === "installing";
  const ready = state.status === "ready";
  const progress = installing ? state.progress : null;
  const ratio = progress && progress.total > 0 ? Math.min(progress.downloaded / progress.total, 1) : 0;
  const percent = Math.round(ratio * 100);
  const indeterminate = progress !== null && progress.phase !== "download";
  // No installer for this platform: nothing to install in-app.
  const canInstall = info.asset !== null;

  return (
    <Dialog open={open} onOpenChange={(next) => (next ? setOpen(true) : onDismiss())}>
      <DialogContent
        // Fixed size: the notes scroll inside, so a long (or still-rendering)
        // changelog never resizes the dialog mid-open.
        className="grid h-[min(560px,85vh)] max-w-lg grid-rows-[auto_minmax(0,1fr)_auto_auto] gap-4"
        onInteractOutside={(e) => installing && e.preventDefault()}
        onEscapeKeyDown={(e) => installing && e.preventDefault()}
      >
        <DialogHeader>
          <DialogTitle className="text-base">发现新版本 v{info.latest}</DialogTitle>
          <DialogDescription>当前版本 v{info.current}</DialogDescription>
        </DialogHeader>

        <ScrollArea className="min-h-0 rounded-lg border border-hairline bg-neutral-50 dark:bg-white/[0.03]">
          <div className="px-4 py-3 text-sm">
            {info.notes?.trim() ? (
              <MemoStreamdown text={info.notes} />
            ) : (
              <span className="text-xs text-neutral-500">此版本没有更新说明。</span>
            )}
          </div>
        </ScrollArea>

        {/* Reserved even when idle, so starting the install does not move the layout. */}
        <div className="flex h-9 flex-col justify-center gap-1.5" role="status" aria-live="polite">
          {progress && (
            <>
              <Progress value={indeterminate ? undefined : percent} />
              <div className="flex justify-between text-xs tabular-nums text-neutral-500">
                <span>{PHASE_LABEL[progress.phase]}</span>
                {progress.phase === "download" && progress.total > 0 && (
                  <span>
                    {mb(progress.downloaded)} / {mb(progress.total)} · {percent}%
                  </span>
                )}
              </div>
            </>
          )}
          {ready && (
            <p className="text-xs text-emerald-600 dark:text-emerald-400" role="status">
              v{info.latest} 已安装完成，重启后生效
            </p>
          )}
          {state.status === "failed" && (
            <p className="line-clamp-2 break-words text-xs text-red-600 dark:text-red-400" role="alert">
              {state.error}
            </p>
          )}
        </div>

        <DialogFooter className="gap-2">
          <Button
            variant="ghost"
            size="sm"
            className="mr-auto gap-1 text-neutral-500"
            onClick={() => void openUrl(info.url ?? "").catch(() => {})}
          >
            在 GitHub 查看
            <ArrowUpRight className="h-3.5 w-3.5" />
          </Button>
          {!installing && !ready && (
            <Button variant="ghost" size="sm" className="text-neutral-500" onClick={onIgnore}>
              忽略此版本
            </Button>
          )}
          {!installing && (
            <Button variant="outline" size="sm" onClick={onDismiss}>
              稍后
            </Button>
          )}
          {ready ? (
            <Button size="sm" onClick={onRelaunch}>
              重启应用
            </Button>
          ) : canInstall ? (
            <Button size="sm" onClick={onInstall} disabled={installing}>
              {installing
                ? "更新中"
                : `${state.status === "failed" ? "重试" : "立即更新"}${info.asset ? ` (${mb(info.asset.size)})` : ""}`}
            </Button>
          ) : (
            <Button size="sm" onClick={() => void openUrl(info.url ?? "").catch(() => {})}>
              前往下载
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
