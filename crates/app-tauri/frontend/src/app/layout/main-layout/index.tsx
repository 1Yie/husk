// App shell — sidebar on the left, `main-col` (title bar + stream +
// composer) on the right. The changes panel lives inside `main-col`,
// below the title bar, so it never covers the window controls.
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
