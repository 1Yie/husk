// MCP pane — the protocol bridges: every manifest that declares an `entry`
// (a server to connect to), their connection state, plus add / edit / delete
// and an explicit reconnect. Host-side capabilities (hooks) are plugins —
// they live on the 插件 page.

import { SkeletonKvRows } from "@/components/ui/skeleton";
import { toast } from "sonner";
import { useState } from "react";
import {
  Plus,
  RefreshCw,
  Server,
  SquarePen,
  Wrench,
} from "@keyline-icons/react";
import { cn } from "@/lib/utils";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { KvList, KvListContent } from "@/components/ui/kv-list";
import { SettingSelect } from "@/features/settings/components/index";
import { DeleteButton } from "@/features/settings/components/delete-button";
import { FormDialog } from "@/features/settings/components/form-dialog";
import { addMcp, probeMcp, reloadMcp, removeMcp, setPluginEnabled, trustMcp, type AgentOverview, type McpProbe, type PluginItem } from "@/lib/agent-ipc/sessions";

/** `entry` is a raw manifest blob — a wasm manifest stores a bare string
 *  (the .wasm filename), only server bridges are objects. */
const entryObj = (p: PluginItem) =>
  p.entry && typeof p.entry === "object" ? p.entry : null;
import { Switch } from "@/components/ui/switch";
import { TooltipSimple } from "@/components/ui/tooltip";
import {
  Chip,
  ChipGroup,
  EmptyState,
  Field,
  ItemDetail,
  ItemIcon,
  ItemRow,
  PaneToolbar,
  ScopeTabs,
  SearchBox,
  Tag,
  matchQuery,
} from "@/features/settings/pages/agent/shared/index";

type Transport = "all" | "stdio" | "http";

/* ============================ MCP ============================ */

export function McpPane({
  ov,
  onReload,
}: {
  ov: AgentOverview | null;
  onReload: () => void;
}) {
  const [filter, setFilter] = useState<Transport>("all");
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  /** Set while the dialog edits an existing plugin — the id names the plugin's
   *  directory, so it stays locked and saving overwrites that manifest. */
  const [editId, setEditId] = useState<string | null>(null);
  /** Which card is expanded — clicking a row reveals the tools it serves. */
  const [expanded, setExpanded] = useState<string | null>(null);
  const [id, setId] = useState("");
  const [name, setName] = useState("");
  const [transport, setTransport] = useState<"stdio" | "http">("stdio");
  const [command, setCommand] = useState("");
  const [args, setArgs] = useState("");
  const [url, setUrl] = useState("");
  const [headers, setHeaders] = useState("");
  const [busy, setBusy] = useState(false);

  /** Results of a MANUAL reconnect, keyed by plugin id. Nothing here is
   *  fetched on mount: the app loads every server once at boot and the overview
   *  carries that live state, so visiting this pane opens no new MCP session.
   *  (It used to probe every plugin on every mount — a fresh handshake per
   *  visit, which tripped the server's rate limit.) */
  const [reconnected, setReconnected] = useState<Record<string, McpProbe>>({});
  const [reconnecting, setReconnecting] = useState<string | null>(null);
  /** Full reload in flight — re-reads every manifest and swaps the live router
   *  so the agent sees servers added/changed since boot. */
  const [reloading, setReloading] = useState(false);

  /** Repo-local plugin awaiting consent — the trust gate is load-level, so an
   *  untrusted repo-local MCP service can't even connect until this passes. */
  const [trusting, setTrusting] = useState<string | null>(null);

  /** Optimistic enable flags — the flag write is a file touch and shouldn't
   *  wait on the MCP reload, so the switch flips instantly and the overview
   *  lands when the background reload finishes. */
  const [optimisticEnabled, setOptimisticEnabled] = useState<Record<string, boolean>>({});
  const pluginEnabled = (p: PluginItem) =>
    optimisticEnabled[p.id] ?? p.enabled ?? true;

  /** Enable/disable — persisted (plugin-state.json), reloads so the bridge's
   *  tools actually stop/start advertising on the next turn. */
  const toggle = async (pluginId: string, enabled: boolean) => {
    setOptimisticEnabled((m) => ({ ...m, [pluginId]: enabled }));
    toast.success(enabled ? `已启用 ${pluginId}` : `已禁用 ${pluginId}`);
    try {
      await setPluginEnabled(pluginId, enabled);
      setReconnected({});
      onReload();
    } catch (e) {
      setOptimisticEnabled((m) => {
        const n = { ...m };
        delete n[pluginId];
        return n;
      });
      toast.error("切换失败", { description: String(e) });
    }
  };

  const trust = async (pluginId: string) => {
    setTrusting(pluginId);
    try {
      await trustMcp(pluginId);
      toast.success(`已信任 ${pluginId}`);
      setReconnected({});
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
      setReconnected({});
      onReload();
    } catch (e) {
      toast.error("重新加载失败", { description: String(e) });
    } finally {
      setReloading(false);
    }
  };

  const reconnect = async (pluginId: string) => {
    setReconnecting(pluginId);
    try {
      const r = await probeMcp(pluginId);
      setReconnected((m) => ({ ...m, [pluginId]: r }));
      if (r.ok) toast.success(`已重连 ${pluginId}`);
      else toast.error("连接失败", { description: r.error });
    } catch (e) {
      toast.error("连接失败", { description: String(e) });
    } finally {
      setReconnecting(null);
    }
  };

  /** Prefill the dialog from the stored manifest and remember which plugin is
   *  being edited (its id is the directory name, so it cannot change). */
  const startEdit = (p: PluginItem) => {
    const http = !!entryObj(p)?.url;
    setEditId(p.id);
    setId(p.id);
    setName(p.name);
    setTransport(http ? "http" : "stdio");
    setUrl(entryObj(p)?.url ?? "");
    setCommand(entryObj(p)?.command ?? "");
    setArgs((entryObj(p)?.args ?? []).join(" "));
    setHeaders(
      Object.entries(entryObj(p)?.headers ?? {})
        .map(([k, v]) => `${k}: ${v}`)
        .join("\n")
    );
    setOpen(true);
  };

  const doDelete = async (pluginId: string) => {
    try {
      await removeMcp(pluginId);
      toast.success("已删除 MCP 插件");
      reload();
    } catch (e) {
      toast.error("删除失败", { description: String(e) });
    }
  };

  const submit = async () => {
    const isHttp = transport === "http";
    if (!id.trim() || (!isHttp && !command.trim()) || (isHttp && !url.trim())) return;
    setBusy(true);
    try {
      const hdrs: Record<string, string> = {};
      if (isHttp && headers.trim()) {
        for (const line of headers.split("\n")) {
          const i = line.indexOf(":");
          if (i > 0) hdrs[line.slice(0, i).trim()] = line.slice(i + 1).trim();
        }
      }
      await addMcp({
        id: id.trim(),
        name: name.trim() || id.trim(),
        transport,
        command: isHttp ? undefined : command.trim(),
        args: isHttp ? undefined : args.trim() ? args.trim().split(/\s+/) : [],
        url: isHttp ? url.trim() : undefined,
        headers: isHttp && Object.keys(hdrs).length ? hdrs : undefined,
      });
      toast.success(editId ? "已更新 MCP 插件" : "已添加 MCP 插件");
      setOpen(false);
      setEditId(null);
      setId(""); setName(""); setCommand(""); setArgs(""); setUrl(""); setHeaders("");
      reload();
    } catch (e) {
      toast.error("添加失败", { description: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const servers = ov?.plugins.filter((p) => entryObj(p)?.command || entryObj(p)?.url) ?? [];
  const isHttp = (p: PluginItem) => !!entryObj(p)?.url;
  const counts: Record<Transport, number> = {
    all: servers.length,
    stdio: servers.filter((p) => !isHttp(p)).length,
    http: servers.filter(isHttp).length,
  };
  const endpoint = (p: PluginItem) =>
    entryObj(p)?.url ??
    `${entryObj(p)?.command ?? ""} ${(entryObj(p)?.args ?? []).join(" ")}`.trim();
  const filtered = servers.filter(
    (p) =>
      (filter === "all" || (filter === "http") === isHttp(p)) &&
      matchQuery(query, p.id, p.name, endpoint(p)),
  );

  const startCreate = () => {
    setEditId(null);
    setId(""); setName(""); setCommand(""); setArgs(""); setUrl(""); setHeaders("");
    setTransport(filter === "http" ? "http" : "stdio");
    setOpen(true);
  };

  return (
    <div className="flex flex-col gap-3">
      <PaneToolbar>
        <ScopeTabs
          value={filter}
          onChange={setFilter}
          options={[
            { value: "all", label: "全部", count: ov ? counts.all : undefined },
            { value: "stdio", label: "stdio", count: ov ? counts.stdio : undefined },
            { value: "http", label: "HTTP", count: ov ? counts.http : undefined },
          ]}
        />
        <SearchBox value={query} onChange={setQuery} placeholder="搜索名称、命令或 URL" />
        <TooltipSimple content="重新读取全部配置并重连">
          <Button
            size="sm"
            variant="outline"
            className="h-7 w-7 shrink-0 p-0"
            aria-label="重新加载"
            disabled={reloading}
            onClick={() => void reload()}
          >
            <RefreshCw className={cn("h-3.5 w-3.5", reloading && "animate-spin")} />
          </Button>
        </TooltipSimple>
        <Button size="sm" variant="outline" className="h-7 shrink-0 text-xs gap-1" onClick={startCreate}>
          <Plus className="h-3.5 w-3.5" /> 添加
        </Button>
      </PaneToolbar>
      <KvList>
        <KvListContent>
          {!ov ? (
            <SkeletonKvRows rows={2} />
          ) : filtered.length ? (
            filtered.map((p) => {
              // Boot state from the overview; a manual reconnect overrides it.
              const re = reconnected[p.id];
              const live = re
                ? { ok: re.ok, version: re.serverVersion ?? "", tools: re.tools ?? [], error: re.error ?? "" }
                : {
                    ok: p.connected,
                    version: p.serverVersion ?? "",
                    tools: p.tools ?? [],
                    error: p.error ?? "",
                  };
              const enabled = pluginEnabled(p);
              const pending = p.repoLocal && !p.trusted;
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
                            !enabled ? "idle" : pending ? "warn" : live.ok ? "success" : "danger"
                          }
                        >
                          <Server className="h-4 w-4" />
                        </ItemIcon>
                      }
                      title={p.name}
                      tags={
                        <>
                          <Tag mono>{isHttp(p) ? "http" : "stdio"}</Tag>
                          {live.version && <Tag mono>v{live.version}</Tag>}
                          {p.repoLocal && <Tag>仓库内</Tag>}
                          {!enabled ? (
                            <Tag>已禁用</Tag>
                          ) : pending ? (
                            <Tag tone="warn">待信任</Tag>
                          ) : live.ok ? (
                            <Tag tone="success" className="tabular-nums">{live.tools.length} 工具</Tag>
                          ) : (
                            <Tag tone="danger">未连接</Tag>
                          )}
                        </>
                      }
                      description={
                        enabled && !pending && !live.ok && live.error ? (
                          <span className="text-red-500" title={live.error}>{live.error}</span>
                        ) : undefined
                      }
                      meta={endpoint(p)}
                      actions={
                        <>
                          {pending && (
                            <TooltipSimple content="仓库内插件在信任前不会加载">
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
                          {!(live.ok && live.tools.length) && (
                            <TooltipSimple content="重连">
                              <Button
                                variant="ghost"
                                size="icon"
                                className="h-6 w-6"
                                aria-label="重连"
                                disabled={reconnecting === p.id}
                                onClick={() => void reconnect(p.id)}
                              >
                                <RefreshCw className={cn("h-3.5 w-3.5", reconnecting === p.id && "animate-spin")} />
                              </Button>
                            </TooltipSimple>
                          )}
                          {/* The span is the tooltip anchor — `asChild` would merge
                              the tooltip's own data-state onto Switch and clobber its
                              `data-[state=checked]` styling (the white-blob bug). */}
                          <TooltipSimple content="禁用后该服务的工具不再提供给模型（持久化）">
                            <span className="mx-1 inline-flex shrink-0">
                              <Switch
                                checked={enabled}
                                onCheckedChange={(v) => void toggle(p.id, v)}
                              />
                            </span>
                          </TooltipSimple>
                          <TooltipSimple content="编辑">
                            <Button variant="ghost" size="icon" className="h-6 w-6" aria-label="编辑"
                              onClick={() => startEdit(p)}>
                              <SquarePen className="h-3.5 w-3.5" />
                            </Button>
                          </TooltipSimple>
                          <DeleteButton
                            confirmTitle="删除 MCP 插件"
                            confirmDescription={`确定要删除「${p.id}」吗？其配置将被移除，此操作无法撤销。`}
                            onConfirm={() => void doDelete(p.id)}
                          />
                        </>
                      }
                    />
                  </CollapsibleTrigger>
                  <CollapsibleContent>
                    <ItemDetail>
                      {live.tools.length ? (
                        <ChipGroup label="工具">
                          {live.tools.map((tool) => (
                            <Chip key={tool} icon={<Wrench className="h-3 w-3" />}>{tool}</Chip>
                          ))}
                        </ChipGroup>
                      ) : (
                        <span className="text-[11px] text-neutral-500">
                          {live.ok ? "该服务没有提供工具" : "尚未连接，点右侧重连"}
                        </span>
                      )}
                    </ItemDetail>
                  </CollapsibleContent>
                </Collapsible>
              );
            })
          ) : (
            <EmptyState
              icon={<Server className="h-4 w-4" />}
              title={query.trim() || filter !== "all" ? "没有匹配的 MCP 服务" : "暂无 MCP 服务"}
              description={
                query.trim() || filter !== "all"
                  ? "调整搜索词或切换传输类型查看"
                  : "接入本地进程或远程端点，把外部工具提供给模型"
              }
              action={
                !query.trim() && filter === "all" && (
                  <Button size="sm" variant="outline" className="h-7 text-xs gap-1" onClick={startCreate}>
                    <Plus className="h-3.5 w-3.5" /> 添加 MCP 服务
                  </Button>
                )
              }
            />
          )}
        </KvListContent>
      </KvList>

      <FormDialog
        open={open}
        onOpenChange={(v: boolean) => {
          setOpen(v);
          if (!v) setEditId(null);
        }}
        title={editId ? "编辑 MCP 插件" : "添加 MCP 插件"}
        width="sm"
        submitLabel={editId ? "保存" : "添加"}
        busy={busy}
        disabled={!id.trim() || (transport === "stdio" ? !command.trim() : !url.trim())}
        onSubmit={() => void submit()}
      >
          <div className="flex flex-col gap-3 py-1">
            <div className="grid grid-cols-2 gap-3">
              <Field label="ID（小写 slug）">
                <Input
                  value={id}
                  onChange={(e) => setId(e.target.value)}
                  placeholder="my-server"
                  disabled={!!editId}
                  className="h-8 text-xs font-mono"
                />
              </Field>
              <Field label="显示名">
                <Input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="默认同 ID"
                  className="h-8 text-xs"
                />
              </Field>
            </div>
            <Field label="传输">
              <SettingSelect
                value={transport}
                onChange={(v) => setTransport(v as "stdio" | "http")}
                options={[
                  { value: "stdio", label: "stdio（本地进程）" },
                  { value: "http", label: "HTTP(S)（远程端点）" },
                ]}
              />
            </Field>
            {transport === "stdio" ? (
              <>
                <Field label="命令">
                  <Input
                    value={command}
                    onChange={(e) => setCommand(e.target.value)}
                    placeholder="npx / uvx / node …"
                    className="h-8 text-xs font-mono"
                  />
                </Field>
                <Field label="参数（空格分隔）">
                  <Input
                    value={args}
                    onChange={(e) => setArgs(e.target.value)}
                    placeholder="-y @modelcontextprotocol/server-xxx"
                    className="h-8 text-xs font-mono"
                  />
                </Field>
              </>
            ) : (
              <>
                <Field label="端点 URL">
                  <Input
                    value={url}
                    onChange={(e) => setUrl(e.target.value)}
                    placeholder="https://mcp.example.com/mcp"
                    className="h-8 text-xs font-mono"
                  />
                </Field>
                <Field label="请求头（每行一个 Key: value，可选）">
                  <Textarea
                    value={headers}
                    onChange={(e) => setHeaders(e.target.value)}
                    rows={3}
                    placeholder={"Authorization: Bearer …\nX-Tenant: acme"}
                    className="font-mono text-[12px] leading-relaxed"
                  />
                </Field>
              </>
            )}
          </div>
      </FormDialog>
    </div>
  );
}
