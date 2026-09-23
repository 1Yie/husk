// App shell — the main window's two-column frame: sidebar on the left,
// `main-col` (title bar + stream + composer) on the right. Pages feed
// both slots; the shell only owns the chrome geometry.
import { cn } from "@/lib/utils";

interface MainLayoutProps {
  sidebar: React.ReactNode;
  children: React.ReactNode;
  className?: string;
}

export function MainLayout({ sidebar, children, className }: MainLayoutProps) {
  return (
    <div
      className={cn(
        "flex h-full w-full bg-white overflow-hidden select-none",
        className
      )}
    >
      {sidebar}
      <div className="main-col h-full overflow-hidden">{children}</div>
    </div>
  );
}
