import { FileDiff, FolderOpen, SquareTerminal } from "lucide-react";
import { useLayoutStore } from "@/app/layout/layout-store";
import { TERMINAL_SHORTCUT } from "@/lib/platform";

export function WorkspaceTools({
  root,
  hasChanges,
  changesDescription,
  onReview,
  onFiles,
  onOpenWorkspace,
  officeMode = false,
}: {
  root: string;
  hasChanges: boolean;
  /** Do not call an unreadable or not-yet-read workspace clean. */
  changesDescription?: string;
  onReview: () => void;
  onFiles: () => void;
  onOpenWorkspace?: () => void;
  officeMode?: boolean;
}) {
  const openTerminal = useLayoutStore((state) => state.setTerminalOpen);
  const row =
    "flex w-full items-start gap-3 px-3 py-3 text-left text-neutral-800 transition-colors enabled:hover:bg-[var(--husk-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--husk-accent)] disabled:cursor-default disabled:opacity-50 motion-reduce:transition-none";
  return (
    <div className="flex min-h-full flex-col justify-center px-5 py-6">
      <div className="mb-4 min-w-0 px-1">
        <h2 className="text-sm font-medium text-neutral-800">工作区工具</h2>
        <p className="mt-1.5 truncate text-[11px] leading-4 text-neutral-500" title={root}>
          {root || "选择项目，或直接打开本机终端"}
        </p>
      </div>
      <div className="divide-y divide-hairline overflow-hidden rounded-lg border border-hairline bg-[var(--husk-bg)]">
        {!officeMode && (
          <button
            type="button"
            className={row}
            aria-label="预览工作区文件"
            disabled={!root && !onOpenWorkspace}
            onClick={() => {
              if (!root) {
                onOpenWorkspace?.();
                return;
              }
              onFiles();
            }}
          >
            <FolderOpen
              size={16}
              className="mt-0.5 flex-none text-neutral-500"
              aria-hidden="true"
            />
            <span className="min-w-0">
              <span className="block text-[13px] font-medium leading-5">
                预览工作区文件
              </span>
              <span className="mt-0.5 block text-[11px] leading-4 text-neutral-600">
                {root ? "浏览代码、Markdown 与图片" : "先选择一个工作区"}
              </span>
            </span>
          </button>
        )}
        {!officeMode && (
          <button
            type="button"
            className={row}
            aria-label="查看工作区改动"
            onClick={onReview}
            disabled={!hasChanges}
          >
            <FileDiff
              size={16}
              className="mt-0.5 flex-none text-neutral-500"
              aria-hidden="true"
            />
            <span className="min-w-0">
              <span className="block text-[13px] font-medium leading-5">
                查看工作区改动
              </span>
              <span className="mt-0.5 block text-[11px] leading-4 text-neutral-600">
                {changesDescription ?? (hasChanges ? "审阅文件差异和未提交内容" : "当前没有未提交改动")}
              </span>
            </span>
          </button>
        )}
        <button
          type="button"
          className={row}
          aria-label="打开终端"
          onClick={() => openTerminal(true)}
        >
          <SquareTerminal
            size={16}
            className="mt-0.5 flex-none text-neutral-500"
            aria-hidden="true"
          />
          <span className="min-w-0 flex-1">
            <span className="block text-[13px] font-medium leading-5">打开终端</span>
            <span className="mt-0.5 flex flex-wrap items-center justify-between gap-x-3 gap-y-1 text-[11px] leading-4 text-neutral-600">
              <span>本机 shell</span>
              {/* Keep the shortcut with the caption so narrow panels can wrap it. */}
              <kbd className="whitespace-nowrap [font-family:inherit] text-[10px] text-neutral-500">
                {TERMINAL_SHORTCUT}
              </kbd>
            </span>
          </span>
        </button>
      </div>
    </div>
  );
}
