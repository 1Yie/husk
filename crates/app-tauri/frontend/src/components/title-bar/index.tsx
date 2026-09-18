// Title bar — custom chrome replacing OS frame.
// Uses window startDragging on mousedown.

import { Minus, Square, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

const win = getCurrentWindow();

export function TitleBar({ workspaceName }: { workspaceName?: string }) {
  const drag = (e: React.MouseEvent) => {
    const el = e.target as HTMLElement;
    if (e.button === 0 && el.closest("[data-drag]") && !el.closest("[data-nodrag]")) {
      void win.startDragging();
    }
  };

  const doubleClick = (e: React.MouseEvent) => {
    const el = e.target as HTMLElement;
    if (el.closest("[data-drag]") && !el.closest("[data-nodrag]")) {
      void win.toggleMaximize();
    }
  };

  return (
    <div
      onMouseDown={drag}
      onDoubleClick={doubleClick}
      data-drag
      className="flex items-center h-9 flex-none bg-neutral-100/80 border-b border-neutral-200 select-none px-2"
    >
      <div data-drag className="flex items-center gap-1.5 px-2">
        <span className="w-2.5 h-2.5 rounded-full bg-neutral-400 inline-block" />
        <span className="text-[12px] font-medium text-neutral-600">
          agent-rs{workspaceName ? ` — ${workspaceName}` : ""}
        </span>
      </div>
      <div data-drag className="flex-1 h-full" />
      <div data-nodrag className="flex items-center gap-0.5">
        <WinButton onClick={() => void win.minimize().catch((e) => console.error("minimize:", e))}>
          <Minus className="h-3.5 w-3.5" />
        </WinButton>
        <WinButton onClick={() => void win.toggleMaximize().catch((e) => console.error("maximize:", e))}>
          <Square className="h-3 w-3" />
        </WinButton>
        <WinButton danger onClick={() => void win.close().catch((e) => console.error("close:", e))}>
          <X className="h-4 w-4" />
        </WinButton>
      </div>
    </div>
  );
}

function WinButton({
  children,
  onClick,
  danger,
}: {
  children: React.ReactNode;
  onClick: () => void;
  danger?: boolean;
}) {
  return (
    <Button
      variant="ghost"
      size="icon-sm"
      onClick={onClick}
      className={cn(
        "text-neutral-500",
        danger
          ? "hover:bg-red-500 hover:text-white"
          : "hover:bg-neutral-200/80 hover:text-neutral-900"
      )}
    >
      {children}
    </Button>
  );
}
