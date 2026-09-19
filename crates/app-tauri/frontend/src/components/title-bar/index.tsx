import { Minus, Square, X } from "@keyline-icons/react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Button } from "@/components/ui/button";
import { isMac } from "@/lib/platform";
import { cn } from "@/lib/utils";

const win = getCurrentWindow();

interface TitleBarProps {
  title?: string;
}

export function TitleBar({ title = "新会话" }: TitleBarProps) {
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
      data-tauri-drag-region
      className="flex items-center h-9 flex-none bg-white border-b border-neutral-200/80 select-none px-3 justify-between"
    >
      {/* macOS: leave room for native traffic-light buttons */}
      {isMac && <div data-drag data-tauri-drag-region className="w-[78px] shrink-0" />}

      {/* Left side: Session Title without any icon */}
      <div data-drag data-tauri-drag-region className="flex items-center min-w-0 max-w-[500px]">
        <span
          className="text-[13px] font-medium text-neutral-800 dark:text-neutral-200 truncate"
          title={title}
        >
          {title}
        </span>
      </div>

      {/* Draggable center area */}
      <div data-drag data-tauri-drag-region className="flex-1 h-full" />

      {/* Right side: Window controls — hidden on macOS (native traffic lights) */}
      {!isMac && (
        <div data-nodrag className="flex items-center gap-0.5 shrink-0">
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
      )}
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
