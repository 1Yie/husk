import { Minus, Square, X } from "@keyline-icons/react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Button } from "@/components/ui/button";
import { isMac } from "@/lib/platform";
import { cn } from "@/lib/utils";

const win = getCurrentWindow();

interface WindowControlsProps {
  /**
   * Whether to render the maximize/restore button. The settings window is
   * created with `.maximizable(false)`, so the button would be dead there —
   * pass `maximize={false}` to hide it.
   */
  maximize?: boolean;
  className?: string;
}

/**
 * Windows-style minimize / maximize / close controls for frameless windows.
 * On macOS the native traffic lights are used, so nothing renders.
 */
export function WindowControls({ maximize = true, className }: WindowControlsProps) {
  if (isMac) return null;
  return (
    <div data-nodrag className={cn("flex items-center gap-0.5 shrink-0", className)}>
      <WinButton onClick={() => void win.minimize().catch((e) => console.error("minimize:", e))}>
        <Minus className="size-[18px]" />
      </WinButton>
      {maximize && (
        <WinButton
          onClick={() => void win.toggleMaximize().catch((e) => console.error("maximize:", e))}
        >
          <Square className="size-[14px]" />
        </WinButton>
      )}
      <WinButton danger onClick={() => void win.close().catch((e) => console.error("close:", e))}>
        <X className="size-[20px]" />
      </WinButton>
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
        danger ? "hover:bg-red-500 hover:text-white" : "hover:bg-neutral-200/80 hover:text-neutral-900"
      )}
    >
      {children}
    </Button>
  );
}
