/** Attached-file chips above the textarea — icon by kind, ✕ removes.
 * Content itself never enters the textarea; it's inlined into the
 * prompt at submit time. */

import { FileText, Image, File, X } from "@keyline-icons/react";
import type { Attachment } from "@/lib/agent-ipc/index";

export function AttachmentChips({
  items,
  onRemove,
}: {
  items: Attachment[];
  onRemove: (path: string) => void;
}) {
  if (items.length === 0) return null;
  return (
    <div className="flex flex-wrap gap-1.5 pb-1">
      {items.map((a) => (
        <span
          key={a.path}
          title={a.path}
          className="inline-flex items-center gap-1.5 max-w-[240px] pl-1.5 pr-1 py-1 rounded-md bg-neutral-100 border border-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] text-[11.5px] text-neutral-700"
        >
          <span className="shrink-0 text-neutral-500">
            {a.kind === "image" && a.data_url ? (
              <img
                src={a.data_url}
                alt={a.name}
                className="h-4 w-4 rounded-sm object-cover"
              />
            ) : a.kind === "image" ? (
              <Image className="h-3.5 w-3.5" />
            ) : a.kind === "text" ? (
              <FileText className="h-3.5 w-3.5" />
            ) : (
              <File className="h-3.5 w-3.5" />
            )}
          </span>
          <span className="truncate">{a.name}</span>
          <button
            type="button"
            onClick={() => onRemove(a.path)}
            className="shrink-0 w-4 h-4 rounded-full flex items-center justify-center text-neutral-500 hover:text-neutral-700 hover:bg-neutral-200 cursor-pointer"
            aria-label={`移除 ${a.name}`}
          >
            <X className="h-2.5 w-2.5" />
          </button>
        </span>
      ))}
    </div>
  );
}
