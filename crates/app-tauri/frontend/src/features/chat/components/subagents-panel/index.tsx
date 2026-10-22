// SubagentsList — the 子代理 view of the session dock: the delegated children
// still out, what each was handed, and a mid-run steer box per child. A running
// child has no row of its own in the stream, so this is where it stays readable
// and reachable. The dock owns the frame.

import { cn } from "@/lib/utils";
import { Orb } from "@/features/chat/components/agent-orb";
import type { RunningChild } from "@/features/chat/hooks/running-children";
import { WAITING_STEER_HINT } from "@/lib/subagents";
import { ChildSteerInput } from "./child-steer";

function ChildRow({ child }: { child: RunningChild }) {
  const waiting = child.waiting != null;
  return (
    <div className="flex w-full flex-col gap-1 rounded-md px-2 py-1.5">
      <div className="flex items-start gap-1.5">
        <Orb
          variant="G2"
          size={16}
          className="mt-0.5 flex-none text-violet-600 dark:text-violet-400"
        />
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-1.5">
            <span
              className="min-w-0 truncate font-mono text-[11.5px] text-neutral-700"
              title={child.name}
            >
              {child.name}
            </span>
            <span
              className={cn(
                "flex-none text-[10.5px]",
                waiting ? "text-amber-600 dark:text-amber-400" : "text-neutral-400",
              )}
            >
              {waiting ? "等待主代理回复" : "运行中"}
            </span>
          </div>
          {/* The task is what a steer answers: three lines here, the rest on
              hover. */}
          <div
            className="line-clamp-3 text-[11px] leading-relaxed text-neutral-500 dark:text-neutral-400"
            title={child.task}
          >
            {child.task}
          </div>
        </div>
      </div>
      {/* A parked child takes the text only after it resumes — saying so beats
          a steer that looks swallowed. */}
      {waiting && (
        <>
          {/* ml-[22px] clears the orb lane, so the block lines up with the
              task above it. */}
          <div className="ml-[22px] rounded-md bg-[color-mix(in_srgb,var(--husk-n100)_70%,transparent)] px-2 py-1 text-[11px] leading-relaxed text-neutral-600 select-text dark:text-neutral-300">
            {child.waiting}
          </div>
          <div className="ml-[22px] text-[10.5px] leading-relaxed text-amber-600 dark:text-amber-400">
            {WAITING_STEER_HINT}
          </div>
        </>
      )}
      <div className="ml-[22px]">
        <ChildSteerInput childId={child.childId} />
      </div>
    </div>
  );
}

/** The 子代理 view's body — only offered while a child is still out. */
export function SubagentsList({ children }: { children: RunningChild[] }) {
  return (
    <div className="w-full">
      {children.map((c) => (
        <ChildRow key={c.childId} child={c} />
      ))}
    </div>
  );
}
