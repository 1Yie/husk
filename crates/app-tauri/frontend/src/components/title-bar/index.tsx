import { getCurrentWindow } from "@tauri-apps/api/window";
import { WindowControls } from "@/components/window-controls";
import { isMac } from "@/lib/platform";

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
      {isMac && <div data-drag data-tauri-drag-region className="w-[78px] shrink-0" />}

      <div data-drag data-tauri-drag-region className="flex items-center min-w-0 max-w-[500px]">
        <span
          className="text-[13px] font-medium text-neutral-800 dark:text-neutral-200 truncate"
          title={title}
        >
          {title}
        </span>
      </div>

      <div data-drag data-tauri-drag-region className="flex-1 h-full" />

      <WindowControls />
    </div>
  );
}
