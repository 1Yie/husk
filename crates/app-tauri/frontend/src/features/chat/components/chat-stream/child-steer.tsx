// Mid-run steer for one delegated child — the capsule's popover is the only
// place a running child can be reached while its card streams in the page.

import { useState } from "react";
import { toast } from "sonner";
import { Input } from "@/components/ui/input";
import { steerChild } from "@/lib/agent-ipc/commands";

export function ChildSteerInput({ childId }: { childId: number }) {
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);

  /** Nothing is appended locally: the kernel echoes an accepted steer back as
   *  `subagent:steer`, and the child's card draws it from there. */
  const send = async () => {
    const text = draft.trim();
    if (!text || sending) return;
    setSending(true);
    try {
      await steerChild(childId, text);
      setDraft("");
    } catch (e) {
      toast.error(`插话失败：${e}`);
    } finally {
      setSending(false);
    }
  };

  return (
    <Input
      value={draft}
      placeholder="插话…"
      aria-label="插话"
      disabled={sending}
      onChange={(e) => setDraft(e.target.value)}
      // The popover acts on Escape — a half-typed steer must not be lost with
      // it, so no key of the box reaches the popover's own handling.
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") {
          e.preventDefault();
          void send();
        }
      }}
      className="h-7 rounded-lg px-2 text-[12px]"
    />
  );
}
