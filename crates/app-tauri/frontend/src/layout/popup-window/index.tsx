// Generic frameless popup window — the shell every secondary Tauri window
// renders inside. Owns the whole chrome so a window page only supplies its
// title and body: drag-region wiring, the macOS traffic-light clearance
// (w-[78px] covers the overlayed red/yellow/green cluster), double-click
// maximize, and the shared WindowControls group. The settings window is a
// consumer: backend creates it .maximizable(false), so it passes
// maximize={false} — a dead button is worse than no button.
import { WindowControls } from "@/components/window-controls";
import { isMac } from "@/lib/platform";
import { cn } from "@/lib/utils";

interface PopupWindowProps {
  /** Left-side title-bar content — icon + text, breadcrumbs… */
  title?: React.ReactNode;
  /** Gate both the maximize/restore button and the double-click gesture. */
  maximize?: boolean;
  className?: string;
  /** Window body — everything below the title bar. */
  children: React.ReactNode;
}

export function PopupWindow({ title, maximize = true, className, children }: PopupWindowProps) {
  return (
    <div
      className={cn(
        "flex flex-col h-screen w-screen bg-neutral-100/50 select-none overflow-hidden font-sans",
        className
      )}
    >
      <div
        data-tauri-drag-region="deep"
        className="flex items-center h-9 flex-none bg-white border-b border-neutral-200/80 select-none px-3 justify-between cursor-default"
      >
        {isMac && <div className="w-[78px] shrink-0" />}

        {title}

        <div className="flex-1 h-full" />

        <WindowControls maximize={maximize} />
      </div>

      {children}
    </div>
  );
}
