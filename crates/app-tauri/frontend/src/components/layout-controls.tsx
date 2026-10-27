import {
  PanelLeftClose,
  PanelLeftOpen,
  PanelRightClose,
  PanelRightOpen,
  SquareTerminal,
} from "lucide-react";
import type { ReactNode } from "react";
import { useLayoutStore } from "@/app/layout/layout-store";
import { useDockStore } from "@/features/chat/components/dock/state";
import { isMac, TERMINAL_SHORTCUT } from "@/lib/platform";
import { cn } from "@/lib/utils";
import { Tooltip, TooltipContent, TooltipSimple, TooltipTrigger } from "@/components/ui/tooltip";

const BUTTON =
  "flex h-7 w-7 flex-none items-center justify-center rounded-md text-neutral-500 transition-colors hover:bg-[var(--husk-hover)] hover:text-neutral-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-400";

export function LeftSidebarToggle() {
  const open = useLayoutStore((state) => state.sidebarOpen);
  const toggle = useLayoutStore((state) => state.toggleSidebar);
  const label = open ? "收起会话侧栏" : "展开会话侧栏";
  const Icon = open ? PanelLeftClose : PanelLeftOpen;
  return (
    // macOS: with the session sidebar collapsed, the title bar starts at the
    // window's edge, under the traffic lights — the same 55px lane the
    // sidebar's header reserves clears them (both inside a `px-3` bar). It
    // eases on the sidebar's own 300ms curve, so the button never passes
    // under the lights mid-slide (the inset dropping at once while the
    // sidebar was still near zero width did exactly that).
    <div
      className={cn(
        "mr-2 flex flex-none items-center",
        isMac &&
          "transition-[padding] duration-300 ease-out motion-reduce:transition-none",
        isMac && !open ? "pl-[55px]" : "pl-0",
      )}
    >
      <TooltipSimple content={label} side="bottom">
        <button
          type="button"
          data-nodrag
          data-tauri-drag-region="false"
          className={BUTTON}
          aria-label={label}
          aria-expanded={open}
          aria-controls="session-navigation"
          onClick={toggle}
        >
          <Icon size={16} aria-hidden="true" />
        </button>
      </TooltipSimple>
    </div>
  );
}

export function RightSidebarToggle() {
  const open = useDockStore((state) => state.open);
  const label = open ? "收起工具侧栏" : "展开工具侧栏";
  const Icon = open ? PanelRightClose : PanelRightOpen;
  return (
    <TooltipSimple content={label} side="bottom">
      <button
        type="button"
        data-nodrag
        data-tauri-drag-region="false"
        className={`${BUTTON} mr-1`}
        aria-label={label}
        aria-expanded={open}
        aria-controls="workspace-tools-sidebar"
        onClick={() => useDockStore.getState().toggle()}
      >
        <Icon size={16} aria-hidden="true" />
      </button>
    </TooltipSimple>
  );
}

/** Both terminal entry points share the wording and shortcut presentation. */
export function TerminalVisibilityTooltip({ open, children }: { open: boolean; children: ReactNode }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent side="top" className="flex items-center gap-2 text-xs">
        <span>{open ? "隐藏终端（保持运行）" : "打开终端"}</span>
        <kbd data-terminal-shortcut className="[font-family:inherit] text-[11px] opacity-60">{TERMINAL_SHORTCUT}</kbd>
      </TooltipContent>
    </Tooltip>
  );
}

/** Subscribe here, not in App or the session list. */
export function TerminalToggle({ open: controlledOpen, onToggle }: { open?: boolean; onToggle: () => void }) {
  const storedOpen = useLayoutStore((state) => state.terminalOpen);
  const open = controlledOpen ?? storedOpen;
  return (
    <TerminalVisibilityTooltip open={open}>
        <button
          type="button"
          onClick={onToggle}
          aria-label={open ? "隐藏终端" : "打开终端"}
          aria-pressed={open}
          className={cn(
            "p-1 rounded transition-colors hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)]",
            open
              ? "bg-[color-mix(in_srgb,var(--husk-black)_6%,transparent)] text-neutral-800"
              : "text-neutral-500 hover:text-neutral-800",
          )}
        >
          <SquareTerminal className="h-4 w-4" aria-hidden="true" />
        </button>
    </TerminalVisibilityTooltip>
  );
}
