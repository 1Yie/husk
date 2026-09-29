// ArtifactsPanel — the office-mode right-side dock: every file the session
// produced (office docs, downloaded assets), with open / save-as / reveal
// actions per row. Fed by `SessionView.artifacts` (applyEvent live,
// viewFromHistory on open) — the office counterpart of `changes`.

import { FileText, FileSpreadsheet, FileImage, FileArchive, File, ExternalLink, Download, FolderOpen, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { toast } from "sonner";
import { openPath, revealPath, saveArtifact } from "@/lib/agent-ipc/index";
import type { Artifact } from "@/features/chat/hooks/stream-view";

export function ExtIcon({ ext }: { ext: string }) {
  const cls = "h-3.5 w-3.5 flex-none";
  switch (ext) {
    case "pptx":
    case "ppt":
      return <FileText className={cn(cls, "text-orange-500")} />;
    case "docx":
    case "doc":
    case "md":
    case "txt":
      return <FileText className={cn(cls, "text-sky-600 dark:text-sky-400")} />;
    case "pdf":
      return <FileText className={cn(cls, "text-red-500 dark:text-red-400")} />;
    case "xlsx":
    case "xls":
    case "csv":
      return <FileSpreadsheet className={cn(cls, "text-emerald-600 dark:text-emerald-400")} />;
    case "png":
    case "jpg":
    case "jpeg":
    case "gif":
    case "webp":
    case "svg":
    case "bmp":
      return <FileImage className={cn(cls, "text-violet-500 dark:text-violet-400")} />;
    case "zip":
    case "tar":
    case "gz":
      return <FileArchive className={cn(cls, "text-neutral-500")} />;
    default:
      return <File className={cn(cls, "text-neutral-500")} />;
  }
}

/** `path/to/file.pptx` → (`path/to/`, `file.pptx`) — same split the
 *  changes rows use (name bold, dir muted). */
function splitPath(p: string): { dir: string; base: string } {
  const i = p.lastIndexOf("/");
  if (i < 0) return { dir: "", base: p };
  return { dir: p.slice(0, i + 1), base: p.slice(i + 1) };
}

function ActionButton({
  label,
  icon: Icon,
  onClick,
}: {
  label: string;
  icon: typeof ExternalLink;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      className="flex-none rounded p-1 text-neutral-400 transition-colors hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] hover:text-neutral-700 dark:hover:text-neutral-200"
    >
      <Icon className="h-3.5 w-3.5" />
    </button>
  );
}

function ArtifactRow({ artifact }: { artifact: Artifact }) {
  const { dir, base } = splitPath(artifact.path);
  const open = () =>
    void openPath(artifact.path).catch((e) => toast.error(`打开失败：${e}`));
  return (
    <div className="w-full">
      <div
        role="button"
        tabIndex={0}
        onClick={open}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            open();
          }
        }}
        className="group relative flex w-full cursor-pointer items-center gap-1.5 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-[color-mix(in_srgb,var(--husk-n200)_45%,transparent)]"
      >
        <span className="flex-none">
          <ExtIcon ext={artifact.ext} />
        </span>
        <span className="min-w-0 flex-1 truncate text-[12px] text-neutral-700 dark:text-neutral-300">
          <span className="font-medium">{base}</span>
          {dir && <span className="ml-1 text-[10.5px] text-neutral-400">{dir}</span>}
        </span>
        {/* Actions float over the row's right edge — absolute + hidden, so
            nothing reflows when they appear (inline reveal twitched the
            whole list). The panel-toned backdrop keeps the name/dir text
            readable underneath. */}
        <span className="absolute right-1 top-1/2 hidden -translate-y-1/2 items-center gap-0.5 rounded-md bg-[color-mix(in_srgb,var(--husk-panel)_85%,transparent)] p-0.5 backdrop-blur-sm group-hover:flex">
          <ActionButton label="打开" icon={ExternalLink} onClick={open} />
          <ActionButton
            label="另存为"
            icon={Download}
            onClick={() =>
              void saveArtifact(artifact.path)
                .then((r) => {
                  if (r.saved) toast.success("已另存", { description: r.path });
                })
                .catch((e) => toast.error(`另存失败：${e}`))
            }
          />
          <ActionButton
            label="在文件夹中显示"
            icon={FolderOpen}
            onClick={() =>
              void revealPath(artifact.path).catch((e) => toast.error(`打开位置失败：${e}`))
            }
          />
        </span>
      </div>
    </div>
  );
}

export function ArtifactsPanel({
  artifacts,
  onClose,
}: {
  artifacts: Artifact[];
  onClose?: () => void;
}) {
  return (
    <aside className="flex h-full w-[300px] flex-none flex-col overflow-hidden border-l border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)] bg-[color-mix(in_srgb,var(--husk-n50)_70%,transparent)] select-none">
      <div className="flex h-9 w-[300px] flex-none items-center gap-2 border-b border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)] px-3">
        <span className="text-[12px] font-medium text-neutral-700 dark:text-neutral-300">
          产物
        </span>
        <span className="text-[10.5px] text-neutral-400">
          {artifacts.length} 个文件
        </span>
        <span className="flex-1" />
        {onClose && (
          <button
            type="button"
            onClick={onClose}
            aria-label="关闭产物面板"
            className="flex-none rounded p-1 text-neutral-500 transition-colors hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] hover:text-neutral-700"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        )}
      </div>

      <div className="w-[300px] flex-1 min-h-0 overflow-y-auto px-1.5 py-1.5">
        {artifacts.length === 0 ? (
          <div className="px-3 py-8 text-center text-[12px] text-neutral-400">
            暂无产物
          </div>
        ) : (
          artifacts.map((a) => <ArtifactRow key={a.path} artifact={a} />)
        )}
      </div>
    </aside>
  );
}
