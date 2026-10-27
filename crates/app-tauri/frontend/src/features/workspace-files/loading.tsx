import { Skeleton } from "@/components/ui/skeleton";
import "./loading.css";

export function DirectoryLoadingSkeleton({ depth = 0, header = false }: { depth?: number; header?: boolean }) {
  return (
    <div className="workspace-files-loading animate-pulse motion-reduce:animate-none"
      role="status" aria-label="正在读取目录" aria-busy="true">
      {header && <div className="workspace-files-loading-header"><Skeleton className="h-3 w-28 rounded-sm" /></div>}
      <div aria-hidden="true" style={{ paddingLeft: 8 + depth * 14, paddingRight: 12 }}>
        {[58, 72, 45, 66, 52, 78].map((width, index) => (
          <div className="workspace-files-loading-row" key={index}>
            <span className="workspace-files-loading-chevron" />
            <Skeleton className="h-3.5 w-3.5 flex-none rounded-sm" />
            <Skeleton className="h-2.5 rounded-sm" style={{ width: `${width}%` }} />
          </div>
        ))}
      </div>
    </div>
  );
}

export function CodeLoadingSkeleton({ label = "正在加载代码" }: { label?: string }) {
  return (
    <div className="workspace-files-code-loading animate-pulse motion-reduce:animate-none"
      role="status" aria-label={label} aria-busy="true">
      {[72, 84, 0, 62, 79, 54, 0, 70, 48, 35].map((width, index) => (
        <div className="workspace-files-code-loading-row" key={index} aria-hidden="true">
          <span className="workspace-files-code-loading-gutter"><Skeleton className="h-2 w-2.5 rounded-sm" /></span>
          {width > 0 && <Skeleton className="h-2.5 rounded-sm" style={{
            width: `${width}%`,
            marginLeft: index === 5 || index === 8 ? 16 : 0,
          }} />}
        </div>
      ))}
    </div>
  );
}
