// ArtifactsList — the 产物 tab body of the unified session dock: every file
// the session produced (office docs, downloaded assets), with open / save-as
// / reveal actions per row. Fed by `SessionView.artifacts` (applyEvent live,
// viewFromHistory on open). The dock owns the frame; this renders rows.

import { FileText, FileSpreadsheet, FileImage, FileArchive, File, ExternalLink, Download, FolderOpen } from "lucide-react";
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
      className="flex h-5 w-5 flex-none items-center justify-center rounded-full text-neutral-500 transition-colors hover:bg-[color-mix(in_srgb,var(--husk-n300)_60%,transparent)] hover:text-neutral-700 dark:hover:text-neutral-200"
    >
      <Icon className="h-3 w-3" />
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
        className="group relative flex h-8 w-full cursor-pointer items-center gap-1.5 rounded-md px-2 text-left transition-colors hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)]"
      >
        <span className="flex-none">
          <ExtIcon ext={artifact.ext} />
        </span>
        <span className="min-w-0 flex-1 truncate text-[12px] text-neutral-700">
          <span className="font-medium">{base}</span>
          {dir && <span className="ml-1 text-[10.5px] text-neutral-400">{dir}</span>}
        </span>
        {/* Actions float over the row's right edge — absolute + hidden, so
            nothing reflows when they appear (inline reveal twitched the
            whole list). The panel-toned backdrop keeps the name/dir text
            readable underneath. */}
        <span className="absolute right-1 top-1/2 hidden -translate-y-1/2 items-center gap-0.5 rounded-full bg-[color-mix(in_srgb,var(--husk-panel)_85%,transparent)] p-0.5 backdrop-blur-sm group-hover:flex">
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

/** The 产物 view's body — one row per file. The dock owns the frame. */
export function ArtifactsList({ artifacts }: { artifacts: Artifact[] }) {
  return (
    <div className="w-full">
      {artifacts.map((a) => (
        <ArtifactRow key={a.path} artifact={a} />
      ))}
    </div>
  );
}
