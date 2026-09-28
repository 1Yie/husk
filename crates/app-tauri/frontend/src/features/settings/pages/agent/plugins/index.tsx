// Plugins pane — the kernel's hook layer. A manifest counts as a plugin when
// it declares lifecycle hooks (`capabilities.hooks` — local commands the
// kernel runs on intercepted events) or has no `entry` at all. MCP is just a
// protocol bridge — a manifest's connection side lives on the MCP page
// (transport, reconnect, tools), its hooks live here.

import { toast } from "sonner";
import { useState } from "react";
import { Plug, RefreshCw, Slash, Wrench, Zap } from "@keyline-icons/react";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Button } from "@/components/ui/button";
import { KvList, KvListContent, KvRow } from "@/components/ui/kv-list";
import { ArmedDeleteButton } from "@/features/settings/components/armed-delete";
import { reloadMcp, removeMcp, setPluginEnabled, trustMcp, type AgentOverview } from "@/lib/agent-ipc/sessions";
import { Switch } from "@/components/ui/switch";
import { TooltipSimple } from "@/components/ui/tooltip";

/* ============================ 插件 ============================ */

export function PluginsPane({
  ov,
  onReload,
}: {
  ov: AgentOverview | null;
  onReload: () => void;
}) {
  /** Which card is expanded — reveals the hooks the plugin would run. */
  const [expanded, setExpanded] = useState<string | null>(null);
  /** Repo-local plugin awaiting consent — its hooks are local commands, so it
   *  stays inert until the user grants trust. */
  const [trusting, setTrusting] = useState<string | null>(null);

  /** Optimistic enable flags — the flag write is a file touch; the reload
   *  runs in the background and the overview lands when it finishes. */
  const [optimisticEnabled, setOptimisticEnabled] = useState<Record<string, boolean>>({});
  const pluginEnabled = (p: AgentOverview["plugins"][number]) =>
    optimisticEnabled[p.id] ?? p.enabled ?? true;

  /** Enable/disable — persisted (plugin-state.json), reloads so hooks
   *  actually stop/start firing on the next turn. */
  const toggle = async (pluginId: string, enabled: boolean) => {
    setOptimisticEnabled((m) => ({ ...m, [pluginId]: enabled }));
    toast.success(enabled ? `已启用 ${pluginId}` : `已禁用 ${pluginId}`);
    try {
      await setPluginEnabled(pluginId, enabled);
      void reload();
    } catch (e) {
      setOptimisticEnabled((m) => {
        const n = { ...m };
        delete n[pluginId];
        return n;
      });
      toast.error("切换失败", { description: String(e) });
    }
  };
  /** Full reload in flight — re-reads every manifest and swaps the live router
   *  so plugins added on disk since boot show up without a restart. */
  const [reloading, setReloading] = useState(false);

  const trust = async (pluginId: string) => {
    setTrusting(pluginId);
    try {
      await trustMcp(pluginId);
      toast.success(`已信任 ${pluginId}`);
      onReload();
    } catch (e) {
      toast.error("信任失败", { description: String(e) });
    } finally {
      setTrusting(null);
    }
  };

  const reload = async () => {
    setReloading(true);
    try {
      const r = await reloadMcp();
      toast.success(`已重新加载 ${(r.plugins ?? []).length} 个插件`);
      onReload();
    } catch (e) {
      toast.error("重新加载失败", { description: String(e) });
    } finally {
      setReloading(false);
    }
  };

  const doDelete = async (pluginId: string) => {
    try {
      await removeMcp(pluginId);
      toast.success("已删除插件");
      void reload();
    } catch (e) {
      toast.error("删除失败", { description: String(e) });
    }
  };

  /** Plugin = declares kernel hooks, or has no server entry at all. A bare
   *  `entry` is just a bridge — it stays on the MCP page. */
  const isPlugin = (p: AgentOverview["plugins"][number]) => {
    const e = p.entry;
    const hasServer =
      typeof e === "object" && e !== null && (!!e.command || !!e.url);
    return (p.hooks?.length ?? 0) > 0 || !hasServer;
  };

  const plugins = ov?.plugins.filter(isPlugin) ?? [];

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-end">
        <Button
          size="sm"
          variant="outline"
          className="h-7 text-xs gap-1"
          disabled={reloading}
          onClick={() => void reload()}
        >
          <RefreshCw className={cn("h-3.5 w-3.5", reloading && "animate-spin")} />
          重新加载
        </Button>
      </div>
      <KvList>
        <KvListContent>
          {plugins.length ? (
            plugins.map((p) => (
              <Collapsible
                key={p.id}
                open={expanded === p.id}
                onOpenChange={(v) => setExpanded(v ? p.id : null)}
              >
                <CollapsibleTrigger asChild>
                  <KvRow
                    label={p.name}
                    description={p.dir}
                    icon={<Plug className="h-4 w-4" />}
                  >
                    <div className="flex items-center gap-2">
                      <Badge variant="outline" className="text-[10px] px-1.5 py-0 h-4">
                        {p.repoLocal ? "仓库内" : "全局"}
                      </Badge>
                      {p.kind && (
                        <Badge variant="outline" className="text-[10px] px-1.5 py-0 h-4">
                          {p.kind}
                        </Badge>
                      )}
                      {p.sandboxed && (
                        <Badge variant="outline" className="text-[10px] px-1.5 py-0 h-4">
                          沙箱
                        </Badge>
                      )}
                      {p.hooks?.length ? (
                        <TooltipSimple
                          className="whitespace-pre-line"
                          content={p.hooks.map((h) => `${h.event}: ${h.command}`).join("\n")}
                        >
                          <span className="text-[11px] text-neutral-500 tabular-nums">
                            {p.hooks.length} 钩子
                          </span>
                        </TooltipSimple>
                      ) : null}
                      {p.tools.length > 0 && (
                        <TooltipSimple
                          className="whitespace-pre-line"
                          content={p.tools.join("\n")}
                        >
                          <span className="text-[11px] text-neutral-500 tabular-nums">
                            {p.tools.length} 工具
                          </span>
                        </TooltipSimple>
                      )}
                      {(p.commandList?.length ?? 0) > 0 && (
                        <span className="text-[11px] text-neutral-500 tabular-nums">
                          {p.commandList!.length} 命令
                        </span>
                      )}
                      {!pluginEnabled(p) && (
                        <Badge variant="outline" className="text-[10px] px-1.5 py-0 h-4">
                          已禁用
                        </Badge>
                      )}
                      {/* span is the anchor — `asChild` merging onto Switch
                          clobbers its `data-[state=checked]` styling. */}
                      <TooltipSimple content="禁用后该插件的钩子不再生效（持久化）">
                        <span className="inline-flex shrink-0">
                          <Switch
                            checked={pluginEnabled(p)}
                            onClick={(e) => e.stopPropagation()}
                            onCheckedChange={(v) => void toggle(p.id, v)}
                          />
                        </span>
                      </TooltipSimple>
                      {p.repoLocal &&
                        (p.trusted ? (
                          <span className="text-[11px] text-neutral-400">已信任</span>
                        ) : (
                          <TooltipSimple content="仓库内插件在信任前不会加载（其钩子会执行本地命令）">
                            <Button
                              variant="ghost"
                              size="sm"
                              className="h-6 px-2 text-[11px]"
                              disabled={trusting === p.id}
                              onClick={(e) => {
                                e.stopPropagation();
                                void trust(p.id);
                              }}
                            >
                              信任
                            </Button>
                          </TooltipSimple>
                        ))}
                      <ArmedDeleteButton stopPropagation onConfirm={() => void doDelete(p.id)} />
                    </div>
                  </KvRow>
                </CollapsibleTrigger>
                <CollapsibleContent>
                  <div className="flex flex-wrap items-center gap-1.5 px-5 pb-3.5">
                    {p.hooks?.map((h, i) => (
                      <TooltipSimple
                        key={`${h.event}-${i}`}
                        content={h.command || "生命周期钩子（本地命令）"}
                      >
                        <Badge
                          variant="outline"
                          className="h-4 px-1.5 font-mono text-[10px] gap-1"
                        >
                          <Zap className="h-3 w-3" />
                          {h.event}
                        </Badge>
                      </TooltipSimple>
                    ))}
                    {p.tools.map((name) => (
                      <Badge
                        key={`tool-${name}`}
                        variant="outline"
                        className="h-4 px-1.5 font-mono text-[10px] gap-1"
                      >
                        <Wrench className="h-3 w-3" />
                        {name}
                      </Badge>
                    ))}
                    {p.commandList?.map((c) => (
                      <TooltipSimple
                        key={`cmd-${c.name}`}
                        content={c.description || c.action}
                      >
                        <Badge
                          variant="outline"
                          className="h-4 px-1.5 font-mono text-[10px] gap-1"
                        >
                          <Slash className="h-3 w-3" />
                          {c.name}
                        </Badge>
                      </TooltipSimple>
                    ))}
                    {!p.hooks?.length &&
                      p.tools.length === 0 &&
                      !p.commandList?.length && (
                      <span className="text-[11px] text-neutral-500">
                        未声明能力
                      </span>
                    )}
                  </div>
                </CollapsibleContent>
              </Collapsible>
            ))
          ) : (
            <KvRow
              label="暂无插件"
              description=""
            />
          )}
        </KvListContent>
      </KvList>
    </div>
  );
}
