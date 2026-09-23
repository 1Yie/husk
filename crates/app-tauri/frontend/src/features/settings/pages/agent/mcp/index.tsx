// MCP pane — discovered servers, live probe, add / edit / delete.

import { toast } from "sonner";
import { useEffect, useState } from "react";
import {
  Plus,
  SquarePen,
  Wrench,
} from "@keyline-icons/react";
import { Badge } from "@/components/ui/badge";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { KvList, KvListContent, KvRow } from "@/components/ui/kv-list";
import { SettingSelect } from "@/features/settings/components/index";
import { ArmedDeleteButton } from "@/features/settings/components/armed-delete";
import { FormDialog } from "@/features/settings/components/form-dialog";
import { addMcp, probeMcp, removeMcp, type AgentOverview, type McpProbe, type PluginItem } from "@/lib/agent-ipc/sessions";
import { Field } from "@/features/settings/pages/agent/shared/index";

/* ============================ MCP ============================ */

export function McpPane({ ov, reload }: { ov: AgentOverview | null; reload: () => void }) {
  const [open, setOpen] = useState(false);
  /** Set while the dialog edits an existing plugin — the id names the plugin's
   *  directory, so it stays locked and saving overwrites that manifest. */
  const [editId, setEditId] = useState<string | null>(null);
  /** Live probe per plugin id: the server's own version + tool names, or the
   *  reason the connection failed. */
  const [probes, setProbes] = useState<Record<string, McpProbe>>({});
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

  const plugins = ov?.plugins ?? [];
  useEffect(() => {
    let alive = true;
    for (const p of plugins) {
      if (p.kind !== "mcp") continue;
      void probeMcp(p.id)
        .then((r) => alive && setProbes((m) => ({ ...m, [p.id]: r })))
        .catch(
          (e) => alive && setProbes((m) => ({ ...m, [p.id]: { ok: false, error: String(e) } }))
        );
    }
    return () => {
      alive = false;
    };
  }, [ov]);

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
            ov.plugins.map((p) => (
              <Collapsible
                key={p.id}
                open={expanded === p.id}
                onOpenChange={(v) => setExpanded(v ? p.id : null)}
              >
                <CollapsibleTrigger asChild>
                  <KvRow
                label={`${p.name}${
                  probes[p.id]?.serverVersion ? ` v${probes[p.id]?.serverVersion}` : ""
                }`}
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
                  {(() => {
                    const probe = probes[p.id];
                    if (!probe)
                      return <span className="text-[11px] text-neutral-400">连接中…</span>;
                    if (!probe.ok)
                      return (
                        <span
                          className="max-w-[240px] truncate text-[11px] text-red-500"
                          title={probe.error}
                        >
                          连接失败：{probe.error}
                        </span>
                      );
                    const names = probe.tools ?? [];
                    return (
                      <span className="text-[11px] text-neutral-500 tabular-nums">
                        {names.length} 工具
                      </span>
                    );
                  })()}
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
                    {(probes[p.id]?.tools ?? []).length ? (
                      (probes[p.id]?.tools ?? []).map((tool) => (
                        <Badge
                          key={tool}
                          variant="secondary"
                          className="h-4 px-1.5 font-mono text-[10px]"
                        >
                          {tool}
                        </Badge>
                      ))
                    ) : (
                      <span className="text-[11px] text-neutral-500">没有可用工具</span>
                    )}
                  </div>
                </CollapsibleContent>
              </Collapsible>
            ))
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
