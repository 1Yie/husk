// MCP pane — discovered servers and the state of the connections the app
// already holds, plus add / edit / delete and an explicit reconnect.

import { toast } from "sonner";
import { useState } from "react";
import {
  Plus,
  RefreshCw,
  SquarePen,
  Wrench,
} from "@keyline-icons/react";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { KvList, KvListContent, KvRow } from "@/components/ui/kv-list";
import { SettingSelect } from "@/features/settings/components/index";
import { ArmedDeleteButton } from "@/features/settings/components/armed-delete";
import { FormDialog } from "@/features/settings/components/form-dialog";
import { addMcp, probeMcp, reloadMcp, removeMcp, type AgentOverview, type McpProbe, type PluginItem } from "@/lib/agent-ipc/sessions";
import { Field } from "@/features/settings/pages/agent/shared/index";

/* ============================ MCP ============================ */

export function McpPane({
  ov,
  onReload,
}: {
  ov: AgentOverview | null;
  onReload: () => void;
}) {
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

  const reload = async () => {
    setReloading(true);
    try {
      const r = await reloadMcp();
      toast.success(`已重新加载 ${(r.plugins ?? []).length} 个插件`);
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
    const http = !!p.entry.url;
    setEditId(p.id);
    setId(p.id);
    setName(p.name);
    setTransport(http ? "http" : "stdio");
    setUrl(p.entry.url ?? "");
    setCommand(p.entry.command ?? "");
    setArgs((p.entry.args ?? []).join(" "));
    setHeaders(
      Object.entries(p.entry.headers ?? {})
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

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-end">
        <Button
          size="sm"
          variant="outline"
          className="h-7 text-xs gap-1"
          onClick={() => setOpen(true)}
        >
          <Plus className="h-3.5 w-3.5" /> 添加
        </Button>
      </div>
      <KvList>
        <KvListContent>
          {ov?.plugins.length ? (
            ov.plugins.map((p) => {
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
              return (
              <Collapsible
                key={p.id}
                open={expanded === p.id}
                onOpenChange={(v) => setExpanded(v ? p.id : null)}
              >
                <CollapsibleTrigger asChild>
                  <KvRow
                label={`${p.name}${live.version ? ` v${live.version}` : ""}`}
                description={
                  p.entry.url ??
                  (p.entry.command
                    ? `${p.entry.command} ${(p.entry.args ?? []).join(" ")}`
                    : p.id)
                }
                icon={<Wrench className="h-4 w-4" />}
              >
                <div className="flex items-center gap-2">
                  <Badge variant="secondary" className="text-[10px] px-1.5 py-0 h-4">
                    {p.entry.url ? "http" : p.kind}
                  </Badge>
                  {live.ok ? (
                    <span className="text-[11px] text-neutral-500 tabular-nums">
                      {live.tools.length} 工具
                    </span>
                  ) : (
                    <span
                      className="max-w-[240px] truncate text-[11px] text-red-500"
                      title={live.error}
                    >
                      {live.error || "未连接"}
                    </span>
                  )}
                  {!(live.ok && live.tools.length) && (
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-6 w-6"
                      title="重连"
                      disabled={reconnecting === p.id}
                      onClick={(e) => {
                        e.stopPropagation();
                        void reconnect(p.id);
                      }}
                    >
                      <RefreshCw className={cn("h-3.5 w-3.5", reconnecting === p.id && "animate-spin")} />
                    </Button>
                  )}
                  <Button variant="ghost" size="icon" className="h-6 w-6" title="编辑"
                    onClick={(e) => { e.stopPropagation(); startEdit(p); }}>
                    <SquarePen className="h-3.5 w-3.5" />
                  </Button>
                  <ArmedDeleteButton stopPropagation onConfirm={() => void doDelete(p.id)} />

                </div>
                  </KvRow>
                </CollapsibleTrigger>
                <CollapsibleContent>
                  <div className="flex flex-wrap gap-1.5 px-5 pb-3.5">
                    {live.tools.length ? (
                      live.tools.map((tool) => (
                        <Badge
                          key={tool}
                          variant="secondary"
                          className="h-4 px-1.5 font-mono text-[10px]"
                        >
                          {tool}
                        </Badge>
                      ))
                    ) : (
                      <span className="text-[11px] text-neutral-500">
                        {live.ok ? "没有可用工具" : "尚未连接，点右侧重连"}
                      </span>
                    )}
                  </div>
                </CollapsibleContent>
              </Collapsible>
              );
            })
          ) : (
            <KvRow label="暂无插件" description="plugins 目录下未发现 manifest.json" />
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
