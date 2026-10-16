import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { ArrowUpRight, BookOpen, Code, Cpu, Globe, Heart, RefreshCw } from "@keyline-icons/react";
import { Skeleton } from "@/components/ui/skeleton";
import { KvList, KvListContent } from "@/components/ui/kv-list";
import { Button } from "@/components/ui/button";
import { ItemIcon, SettingsSection, Tag } from "@/features/settings/components";
import { openUrl } from "@/lib/agent-ipc/sessions";
import { useUpdateStore } from "@/features/update/update-store";
import { cn } from "@/lib/utils";
import { toast } from "sonner";
import iconUrl from "@/assets/husk-icon.png";

const REPO_URL = "https://github.com/1Yie/husk";
const LINKS = [
  { title: "官网", detail: "husk.ingstar.im", url: "https://husk.ingstar.im", Icon: Globe },
  { title: "使用文档", detail: "husk.ingstar.im/docs", url: "https://husk.ingstar.im/docs", Icon: BookOpen },
  { title: "源代码", detail: "github.com/1Yie/husk", url: REPO_URL, Icon: Code },
  { title: "参与贡献", detail: "反馈问题或提交改进", url: `${REPO_URL}/issues`, Icon: Heart },
  { title: "Korei 内核", detail: "github.com/1Yie/korei", url: "https://github.com/1Yie/korei", Icon: Cpu },
] as const;

export function AboutSettings() {
  const [version, setVersion] = useState("");
  const checking = useUpdateStore((s) => s.checking);
  const [updateStatus, setUpdateStatus] = useState("");

  useEffect(() => {
    let alive = true;
    void getVersion()
      .then((v) => { if (alive) setVersion(v); })
      .catch(() => {});
    return () => { alive = false; };
  }, []);

  const visit = (url: string) => {
    void openUrl(url).catch((e) => toast.error("打开链接失败", { description: String(e) }));
  };

  const handleCheck = async () => {
    if (checking) return;
    try {
      const r = await useUpdateStore.getState().check();
      if (!r) return;
      if (r.is_newer && r.latest) {
        // The shared dialog opens with the notes and the install button.
        setUpdateStatus(`新版本 v${r.latest} 可用`);
      } else if (r.latest) {
        setUpdateStatus("已是最新版本");
        toast.info(`已是最新版本 v${r.current}`);
      } else {
        setUpdateStatus("尚无正式发布版");
        toast.info("尚无正式发布版");
      }
    } catch (e) {
      setUpdateStatus("检查更新失败");
      toast.error("检查更新失败", { description: String(e) });
    }
  };

  return (
    <div className="flex min-w-0 flex-col gap-8">
      <KvList>
        <div className="flex flex-wrap items-center gap-5 p-6">
          <img src={iconUrl} alt="" className="h-20 w-20 shrink-0" draggable={false} />
          <div className="flex min-w-0 flex-1 flex-col gap-2">
            <div className="flex flex-wrap items-center gap-2.5">
              <span className="text-2xl font-semibold tracking-tight text-neutral-900">Husk</span>
              {version ? <Tag mono>v{version}</Tag> : <Skeleton className="h-4 w-14 animate-pulse" />}
            </div>
            <span className="text-xs text-neutral-500">由 Korei 驱动</span>
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-3 border-t border-hairline px-5 py-3.5">
          <ItemIcon className="mt-0"><RefreshCw className="h-4 w-4" /></ItemIcon>
          <span className="min-w-0 flex-1 text-xs text-neutral-500" role="status">
            {checking ? "正在检查更新…" : updateStatus || "版本更新"}
          </span>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={checking}
            onClick={() => void handleCheck()}
            className="ml-auto h-7 text-xs"
          >
            <RefreshCw className={cn("h-3.5 w-3.5", checking && "animate-spin")} />
            {checking ? "检查中…" : "检查更新"}
          </Button>
        </div>
      </KvList>

      <SettingsSection title="了解 Husk">
        <KvList>
          <KvListContent>
            {LINKS.map(({ title, detail, url, Icon }) => (
              <button
                key={title}
                type="button"
                onClick={() => visit(url)}
                className="group flex w-full items-center gap-3 px-5 py-4 text-left transition-colors hover:bg-neutral-50 focus-visible:bg-neutral-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent"
              >
                <ItemIcon className="mt-0"><Icon className="h-4 w-4" /></ItemIcon>
                <span className="flex min-w-0 flex-1 flex-col gap-1">
                  <span className="text-sm text-neutral-800">{title}</span>
                  <span className="truncate text-xs text-neutral-500">{detail}</span>
                </span>
                <ArrowUpRight className="h-3.5 w-3.5 shrink-0 text-neutral-400 transition-colors group-hover:text-neutral-700" />
              </button>
            ))}
          </KvListContent>
        </KvList>
      </SettingsSection>
    </div>
  );
}
