// Plugins pane — the kernel's hook layer. A manifest counts as a plugin when
// it declares lifecycle hooks (`capabilities.hooks` — local commands the
// kernel runs on intercepted events) or has no `entry` at all. MCP is just a
// protocol bridge — a manifest's connection side lives on the MCP page
// (transport, reconnect, tools), its hooks live here.

import { SkeletonKvRows } from "@/components/ui/skeleton";
import { toast } from "sonner";
import { useState } from "react";
import { Plug, RefreshCw, Slash, Wrench, Zap } from "@keyline-icons/react";
import { cn } from "@/lib/utils";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Button } from "@/components/ui/button";
import { KvList, KvListContent } from "@/components/ui/kv-list";
import { ArmedDeleteButton } from "@/features/settings/components/armed-delete";
import { reloadMcp, removeMcp, setPluginEnabled, trustMcp, type AgentOverview } from "@/lib/agent-ipc/sessions";
import { Switch } from "@/components/ui/switch";
import { TooltipSimple } from "@/components/ui/tooltip";
import {
  Chip,
  ChipGroup,
  EmptyState,
  ItemDetail,
  ItemIcon,
  ItemRow,
  PaneToolbar,
  ScopeTabs,
  SearchBox,
  Tag,
  matchQuery,
} from "@/features/settings/pages/agent/shared/index";

type Scope = "all" | "global" | "repo";

/* ============================ 插件 ============================ */

export function PluginsPane({
  ov,
  onReload,
}: {
  ov: AgentOverview | null;
  onReload: () => void;
}) {
  const [scope, setScope] = useState<Scope>("all");
  const [query, setQuery] = useState("");
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
      await reloadMcp();
      toast.success("已重新加载");
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
  const counts: Record<Scope, number> = {
    all: plugins.length,
    global: plugins.filter((p) => !p.repoLocal).length,
    repo: plugins.filter((p) => p.repoLocal).length,
  };
  const filtered = plugins.filter(
    (p) =>
      (scope === "all" || (scope === "repo" ? p.repoLocal : !p.repoLocal)) &&
      matchQuery(query, p.id, p.name, p.dir),
  );

  return (
    <div className="flex flex-col gap-3">
      <PaneToolbar>
        <ScopeTabs
          value={scope}
          onChange={setScope}
          options={[
            { value: "all", label: "全部", count: ov ? counts.all : undefined },
            { value: "global", label: "全局", count: ov ? counts.global : undefined },
            { value: "repo", label: "仓库内", count: ov ? counts.repo : undefined },
          ]}
        />
        <SearchBox value={query} onChange={setQuery} placeholder="搜索名称或目录" />
        <Button
          size="sm"
          variant="outline"
          className="h-7 shrink-0 text-xs gap-1"
          disabled={reloading}
          onClick={() => void reload()}
        >
          <RefreshCw className={cn("h-3.5 w-3.5", reloading && "animate-spin")} />
          重新加载
        </Button>
      </PaneToolbar>
      <KvList>
        <KvListContent>
          {!ov ? (
            <SkeletonKvRows rows={3} />
          ) : filtered.length ? (
            filtered.map((p) => {
              const enabled = pluginEnabled(p);
              const pending = p.repoLocal && !p.trusted;
              const hooks = p.hooks ?? [];
              const commands = p.commandList ?? [];
              const summary = [
                hooks.length ? `${hooks.length} 钩子` : null,
                p.tools.length ? `${p.tools.length} 工具` : null,
                commands.length ? `${commands.length} 命令` : null,
              ].filter(Boolean).join(" · ");
              return (
                <Collapsible
                  key={p.id}
                  open={expanded === p.id}
                  onOpenChange={(v) => setExpanded(v ? p.id : null)}
                >
                  <CollapsibleTrigger asChild>
                    <ItemRow
                      expanded={expanded === p.id}
                      icon={
                        <ItemIcon
                          status={
                            !enabled ? "idle" : pending ? "warn" : p.error ? "danger" : "success"
                          }
                        >
                          <Plug className="h-4 w-4" />
                        </ItemIcon>
                      }
                      title={p.name}
                      tags={
                        <>
                          <Tag>{p.repoLocal ? "仓库内" : "全局"}</Tag>
                          {p.kind && <Tag mono>{p.kind}</Tag>}
                          {p.sandboxed && <Tag>沙箱</Tag>}
                          {!enabled ? (
                            <Tag>已禁用</Tag>
                          ) : pending ? (
                            <Tag tone="warn">待信任</Tag>
                          ) : p.error ? (
                            <TooltipSimple content={p.error}>
                              <span className="inline-flex">
                                <Tag tone="danger">加载失败</Tag>
                              </span>
                            </TooltipSimple>
                          ) : null}
                        </>
                      }
                      description={summary}
                      emptyDescription="未声明能力"
                      meta={p.dir}
                      actions={
                        <>
                          {pending && (
                            <TooltipSimple content="仓库内插件在信任前不会加载（其钩子会执行本地命令）">
                              <Button
                                variant="ghost"
                                size="sm"
                                className="h-6 px-2 text-[11px] text-amber-600"
                                disabled={trusting === p.id}
                                onClick={() => void trust(p.id)}
                              >
                                信任
                              </Button>
                            </TooltipSimple>
                          )}
                          {/* span is the anchor — `asChild` merging onto Switch
                              clobbers its `data-[state=checked]` styling. */}
                          <TooltipSimple content="禁用后该插件的钩子不再生效（持久化）">
                            <span className="mx-1 inline-flex shrink-0">
                              <Switch
                                checked={enabled}
                                onCheckedChange={(v) => void toggle(p.id, v)}
                              />
                            </span>
                          </TooltipSimple>
                          <ArmedDeleteButton onConfirm={() => void doDelete(p.id)} />
                        </>
                      }
                    />
                  </CollapsibleTrigger>
                  <CollapsibleContent>
                    <ItemDetail>
                      {hooks.length > 0 && (
                        <ChipGroup label="钩子">
                          {hooks.map((h, i) => (
                            <TooltipSimple key={`${h.event}-${i}`} content={h.command || "生命周期钩子（本地命令）"}>
                              <span className="inline-flex">
                                <Chip icon={<Zap className="h-3 w-3" />}>{h.event}</Chip>
                              </span>
                            </TooltipSimple>
                          ))}
                        </ChipGroup>
                      )}
                      {p.tools.length > 0 && (
                        <ChipGroup label="工具">
                          {p.tools.map((name) => (
                            <Chip key={name} icon={<Wrench className="h-3 w-3" />}>{name}</Chip>
                          ))}
                        </ChipGroup>
                      )}
                      {commands.length > 0 && (
                        <ChipGroup label="命令">
                          {commands.map((c) => (
                            <TooltipSimple key={c.qualified} content={c.description || c.action}>
                              <span className="inline-flex">
                                <Chip icon={<Slash className="h-3 w-3" />}>{c.name}</Chip>
                              </span>
                            </TooltipSimple>
                          ))}
                        </ChipGroup>
                      )}
                      {!summary && (
                        <span className="text-[11px] text-neutral-500">该插件未声明钩子、工具或命令</span>
                      )}
                    </ItemDetail>
                  </CollapsibleContent>
                </Collapsible>
              );
            })
          ) : (
            <EmptyState
              icon={<Plug className="h-4 w-4" />}
              title={query.trim() ? "没有匹配的插件" : scope === "all" ? "暂无插件" : scope === "repo" ? "仓库内暂无插件" : "全局暂无插件"}
              description={
                query.trim() || scope !== "all"
                  ? "调整搜索词或切换范围查看"
                  : "将插件放入插件目录后，点击「重新加载」"
              }
            />
          )}
        </KvListContent>
      </KvList>
    </div>
  );
}
