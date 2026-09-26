// About pane — app icon + name + version, then a kv-list of meta rows
// (author / repo / contribute / update check). Version reads at runtime
// from the Tauri package version so it can never drift from the build.
import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import {
  User,
  Code,
  Heart,
  RefreshCw,
  ArrowUpRight,
} from "@keyline-icons/react";
import { KvList, KvListContent, KvRow } from "@/components/ui/kv-list";
import { Button } from "@/components/ui/button";
import { openUrl, checkUpdate } from "@/lib/agent-ipc/sessions";
import { toast } from "sonner";
import iconUrl from "@/assets/husk-icon.png";

const REPO_URL = "https://github.com/1Yie/husk";
const AUTHOR_URL = "https://github.com/1Yie";

export function AboutSettings() {
  const [version, setVersion] = useState("");
  const [checking, setChecking] = useState(false);
  useEffect(() => {
    void getVersion()
      .then(setVersion)
      .catch(() => {});
  }, []);

  const handleCheck = async () => {
    setChecking(true);
    try {
      const r = await checkUpdate();
      if (r.is_newer && r.latest && r.url) {
        toast.success(`发现新版本 v${r.latest}`, {
          description: "点击前往下载页",
          action: { label: "前往下载", onClick: () => void openUrl(r.url!) },
        });
      } else if (r.latest) {
        toast.info(`已是最新版本 v${r.current}`, {
          description: "当前使用的即是最新发布版",
        });
      } else {
        toast.info(`当前版本 v${r.current}`, {
          description: "尚未发布任何正式版本",
        });
      }
    } catch (e) {
      toast.error("检查更新失败", { description: String(e) });
    } finally {
      setChecking(false);
    }
  };

  return (
    <div className="flex flex-col items-center gap-8 pt-16 select-none">
      <div className="flex flex-col items-center gap-4">
        <img
          src={iconUrl}
          alt="Husk"
          className="h-28 w-28"
          draggable={false}
        />
        <div className="flex flex-col items-center gap-1.5">
          <span className="text-xl font-semibold text-neutral-900 tracking-tight">
            Husk
          </span>
          <span className="text-[13px] text-neutral-500 font-mono tabular-nums">
            {version ? `v${version}` : "…"}
          </span>
        </div>
      </div>

      <KvList className="w-full max-w-md">
        <KvListContent>
          <KvRow label="作者" icon={<User className="h-4 w-4" />}>
            <button
              type="button"
              onClick={() => void openUrl(AUTHOR_URL)}
              className="flex items-center gap-1 text-[13px] text-accent hover:underline cursor-pointer"
            >
              1Yie
              <ArrowUpRight className="h-3.5 w-3.5" />
            </button>
          </KvRow>

          <KvRow label="开源地址" icon={<Code className="h-4 w-4" />}>
            <button
              type="button"
              onClick={() => void openUrl(REPO_URL)}
              className="flex items-center gap-1 text-[13px] text-accent hover:underline cursor-pointer"
            >
              github.com/1Yie/husk
              <ArrowUpRight className="h-3.5 w-3.5" />
            </button>
          </KvRow>

          <KvRow
            label="参与贡献"
            description="提交 Issue 或 Pull Request"
            icon={<Heart className="h-4 w-4" />}
          >
            <button
              type="button"
              onClick={() => void openUrl(`${REPO_URL}/issues`)}
              className="flex items-center gap-1 text-[13px] text-accent hover:underline cursor-pointer"
            >
              GitHub Issues
              <ArrowUpRight className="h-3.5 w-3.5" />
            </button>
          </KvRow>

          <KvRow
            label="检查更新"
            description={version ? `当前版本 v${version}` : "当前版本 …"}
            icon={<RefreshCw className="h-4 w-4" />}
          >
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={checking}
              onClick={() => void handleCheck()}
              className="h-7 px-3 rounded-lg text-[12px]"
            >
              {checking ? "检查中…" : "检查更新"}
            </Button>
          </KvRow>
        </KvListContent>
      </KvList>
    </div>
  );
}
