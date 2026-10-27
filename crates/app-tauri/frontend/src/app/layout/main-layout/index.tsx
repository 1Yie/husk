// App shell — sidebar on the left, `main-col` (title bar + stream +
// composer) on the right. The changes panel lives inside `main-col`,
// below the title bar, so it never covers the window controls.
import { cn } from "@/lib/utils";
import { useEffect } from "react";
import { useLayoutStore } from "../layout-store";

interface MainLayoutProps {
  sidebar: React.ReactNode;
  children: React.ReactNode;
  terminal?: React.ReactNode;
  className?: string;
}

export function MainLayout({ sidebar, children, terminal, className }: MainLayoutProps) {
  const sidebarOpen = useLayoutStore((state) => state.sidebarOpen);
  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.shiftKey && !event.altKey
        && event.key.toLowerCase() === "j" && !event.repeat) {
        event.preventDefault();
        event.stopPropagation();
        useLayoutStore.getState().toggleTerminal();
      }
    };
    window.addEventListener("keydown", keydown, true);
    return () => window.removeEventListener("keydown", keydown, true);
  }, []);
  return (
    <div
      className={cn(
        "flex h-full w-full bg-white overflow-hidden select-none",
        className
      )}
    >
      <div id="session-navigation" aria-hidden={!sidebarOpen}
        {...(!sidebarOpen ? { inert: "" } : {})}
        className="h-full flex-none overflow-hidden transition-[width] duration-300 ease-out motion-reduce:transition-none"
        style={{ width: sidebarOpen ? 240 : 0 }}>
        <div className={cn("h-full w-[240px] transition-transform duration-300 ease-out motion-reduce:transition-none",
          sidebarOpen ? "translate-x-0" : "-translate-x-full")}>
          {sidebar}
        </div>
      </div>
      <div className="main-col isolate h-full overflow-hidden">{children}{terminal}</div>
    </div>
  );
}
