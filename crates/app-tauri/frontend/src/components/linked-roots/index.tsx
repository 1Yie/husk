/** Linked workspaces — one grant as a row.
 *
 *  The composer's grant menu is the only surface, so the row is one line:
 *  name, which layer granted it, path, the write switch, remove. The data
 *  (and every mutation) lives in `lib/linked-roots`. */
import { X } from "@keyline-icons/react";
import { linkedName } from "@/lib/linked-roots";
import type { LinkedRoot } from "@/lib/agent-ipc/index";
import { cn } from "@/lib/utils";

export function LinkedRootRow({
  root,
  busy,
  onAccess,
  onRemove,
}: {
  root: LinkedRoot;
  busy?: boolean;
  onAccess: (root: LinkedRoot, access: LinkedRoot["access"]) => void;
  onRemove: (path: string) => void;
}) {
  const writable = root.access === "readwrite";
  return (
    <div className="flex min-w-0 items-center gap-1.5 rounded-lg px-2 py-1 hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)]">
      <span className="max-w-[7em] shrink-0 truncate text-xs font-medium text-neutral-800">
        {linkedName(root.path)}
      </span>
      <span className="shrink-0 rounded bg-[color-mix(in_srgb,var(--husk-black)_6%,transparent)] px-1 text-[10px] text-neutral-500">
        {root.scope === "workspace" ? "本工作区" : "本会话"}
      </span>
      <span className="min-w-0 flex-1 truncate text-[10px] text-neutral-400" title={root.path}>
        {root.path}
      </span>
      {/* Two tabs, one of them lit — the same segmented shape as the scope
          control below. A switch would hide what the other position means;
          spelling both out makes the grant readable and changeable in one
          click. */}
      <div className="flex shrink-0 items-center gap-0.5 rounded-full bg-[color-mix(in_srgb,var(--husk-black)_6%,transparent)] p-0.5">
        {(["read", "readwrite"] as const).map((access) => {
          const active = root.access === access;
          return (
            <button
              key={access}
              type="button"
              aria-disabled={busy}
              aria-pressed={active}
              onClick={() => onAccess(root, access)}
              className={cn(
                "cursor-pointer rounded-full px-1.5 py-0.5 text-[10px] transition-colors",
                active
                  ? "bg-white text-neutral-900 shadow-2xs"
                  : "text-neutral-500 hover:text-neutral-900",
              )}
            >
              {access === "read" ? "只读" : "读写"}
            </button>
          );
        })}
      </div>
      <button
        type="button"
        aria-disabled={busy}
        aria-label="取消关联"
        onClick={() => onRemove(root.path)}
        className="shrink-0 cursor-pointer rounded p-0.5 text-neutral-400 transition-colors hover:bg-[color-mix(in_srgb,var(--husk-black)_8%,transparent)] hover:text-neutral-700"
      >
        <X className="h-3 w-3" />
      </button>
    </div>
  );
}
