// 智能体 panes — one page per nav item under the "智能体" group. Read +
// write surfaces: instructions editing, model list (+add), skills (+add),
// MCP plugins (+add), subagents (builtin + custom, +add).
import { useEffect, useState } from "react";
import {
  Cpu,
  Sparkles,
  Wrench,
  Bot,
  Plus,
  SquarePen,
  Layers,
  Brain,
  Coins,
} from "@keyline-icons/react";
import { KvList, KvListContent, KvRow } from "@/components/ui/kv-list";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { SettingSelect } from "../../components/settings";
import {
  getAgentOverview,
  getInstructions,
  setInstructions,
  createSkill,
  addMcp,
  createSubagent,
  getAppConfig,
  saveAppConfig,
  type AgentOverview,
  type InstructionsSet,
} from "../../invoke/agent/sessions";

export type AgentTab = "instructions" | "model" | "skills" | "mcp" | "subagent";

/** Field label + control, dialog rows. */
function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <Label className="text-xs text-neutral-600">{label}</Label>
      {children}
    </div>
  );
}

/* ============================ 指令 ============================ */

function InstructionsPane() {
  const [data, setData] = useState<InstructionsSet | null>(null);
  const [drafts, setDrafts] = useState({ global: "", workspace: "" });
  const [saving, setSaving] = useState<string | null>(null);

  useEffect(() => {
    void getInstructions()
      .then((d) => {
        setData(d);
        setDrafts({ global: d.global.content, workspace: d.workspace.content });
      })
      .catch(() => {});
  }, []);

  const save = async (scope: "global" | "workspace") => {
    setSaving(scope);
    try {
      await setInstructions(scope, drafts[scope]);
      toast.success("已保存");
    } catch (e) {
      toast.error("保存失败", { description: String(e) });
    } finally {
      setSaving(null);
    }
  };

  const Block = ({
    scope,
    title,
    file,
  }: {
    scope: "global" | "workspace";
    title: string;
    file?: { path: string | null };
  }) => (
    <div className="flex flex-col gap-2">
      <div className="flex items-baseline justify-between gap-2">
        <span className="text-sm font-semibold text-neutral-900">{title}</span>
        {file?.path && (
          <span className="text-[11px] text-neutral-500 font-mono truncate">
            {file.path}
          </span>
        )}
      </div>
      <Textarea
        value={drafts[scope]}
        onChange={(e) => setDrafts((d) => ({ ...d, [scope]: e.target.value }))}
        rows={10}
        className="font-mono text-[12px] leading-relaxed resize-y min-h-40"
        placeholder="在此编写自定义指令，会追加到每个新会话的系统提示词末尾"
      />
      <div className="flex justify-end">
        <Button
          size="sm"
          variant="outline"
          className="h-7 text-xs"
          disabled={saving === scope}
          onClick={() => void save(scope)}
        >
          {saving === scope ? "保存中…" : "保存"}
        </Button>
      </div>
    </div>
  );

  return (
    <div className="flex flex-col gap-8">
      <Block scope="global" title="全局指令" file={data?.global} />
      <Block scope="workspace" title="项目指令" file={data?.workspace} />
    </div>
  );
}

/* ============================ 模型 ============================ */

/** Provider kinds the config schema accepts — labeled by their real API
 * names (openai_compat IS the Chat Completions API). */
const KIND_OPTIONS = [
  { value: "openai_compat", label: "OpenAI Chat Completions" },
  { value: "openai_responses", label: "OpenAI Responses" },
  { value: "anthropic", label: "Anthropic (Claude)" },
  { value: "gemini", label: "Google Gemini" },
];

const DEFAULT_BASES: Record<string, string> = {
  openai_compat: "https://api.openai.com/v1",
  openai_responses: "https://api.openai.com/v1",
  anthropic: "https://api.anthropic.com",
  gemini: "https://generativelanguage.googleapis.com/v1beta",
};

/** Write `value` into `obj` under whichever of `keys` already exists
 * (config files mix snake_case and camelCase aliases) — else canonical. */
function setAliased(obj: Record<string, any>, keys: string[], value: any) {
  const hit = keys.find((k) => k in obj) ?? keys[0];
  obj[hit] = value;
}

/** Normalize a model entry — `Simple` strings become `{id}` objects so the
 * editor always writes a rich entry. */
function normModel(m: any): Record<string, any> {
  return typeof m === "string" ? { id: m } : (m ?? {});
}

const numOr = (v: string) => {
  const n = Number(v);
  return v.trim() !== "" && !isNaN(n) ? n : undefined;
};
const THINK_LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh", "max"] as const;

const triOr = (v: string) =>
  v === "true" ? true : v === "false" ? false : undefined;

function ModelPane({ ov, reload }: { ov: AgentOverview | null; reload: () => void }) {
  const [cfg, setCfg] = useState<Record<string, any> | null>(null);
  const [cfgPath, setCfgPath] = useState("");
  const [busy, setBusy] = useState(false);
  // Armed-delete — destructive buttons need a second click within 3s.
  const [delArmed, setDelArmed] = useState(false);
  const armThenRun = (fn: () => void) => () => {
    if (!delArmed) {
      setDelArmed(true);
      window.setTimeout(() => setDelArmed(false), 3000);
      return;
    }
    setDelArmed(false);
    fn();
  };

  // ---- add-provider dialog ----
  const [provOpen, setProvOpen] = useState(false);
  const [provKey, setProvKey] = useState("");
  const [provKind, setProvKind] = useState("openai_compat");
  const [provBase, setProvBase] = useState("");
  const [provKey_, setProvKey_] = useState(""); // api key (name clash w/ provKey)
  const [provModels, setProvModels] = useState("");
  const [provDefault, setProvDefault] = useState("");

  // ---- edit-provider dialog ----
  const [nStore, setNStore] = useState("default");
  const [nDev, setNDev] = useState("default");
  const [nEff, setNEff] = useState("default");
  const [nTok, setNTok] = useState("");
  const [nHeaders, setNHeaders] = useState("");
  const [eHeaders, setEHeaders] = useState("");
  const [editProv, setEditProv] = useState<string | null>(null);
  const [eBase, setEBase] = useState("");
  const [eKey, setEKey] = useState("");
  const [eDefault, setEDefault] = useState("");
  const [eStore, setEStore] = useState("default");
  const [eDevRole, setEDevRole] = useState("default");
  const [eEffort, setEEffort] = useState("default");
  const [eMaxTok, setEMaxTok] = useState("");

  // ---- edit-model dialog ----
  const [editModel, setEditModel] = useState<{ provider: string; index: number } | null>(null);
  const [mId, setMId] = useState("");
  const [mName, setMName] = useState("");
  const [mCtx, setMCtx] = useState("");
  const [mReasoning, setMReasoning] = useState("default");
  const [mInput, setMInput] = useState("");
  const [mCostIn, setMCostIn] = useState("");
  const [mCostOut, setMCostOut] = useState("");
  const [mCostCr, setMCostCr] = useState("");
  const [mCostCw, setMCostCw] = useState("");
  const [mThink, setMThink] = useState<Record<string, string>>({});

  const loadCfg = () => {
    void getAppConfig()
      .then((ac) => {
        setCfg(ac.config as any);
        setCfgPath(ac.path ?? "");
      })
      .catch(() => {});
  };
  useEffect(loadCfg, []);

  const persist = async (next: Record<string, any>) => {
    setBusy(true);
    try {
      await saveAppConfig(next);
      setCfg(next);
      reload();
      return true;
    } catch (e) {
      toast.error("保存失败", { description: String(e) });
      return false;
    } finally {
      setBusy(false);
    }
  };

  /* ---------- provider: add ---------- */
  const submitAddProvider = async () => {
    if (!provKey.trim() || !provBase.trim() || !cfg) return;
    const k = provKey.trim().toLowerCase();
    if (cfg.providers?.[k]) {
      toast.error("该提供商已存在");
      return;
    }
    const models = provModels.split(",").map((m) => m.trim()).filter(Boolean);
    const p: Record<string, any> = {
      kind: provKind,
      base_url: provBase.trim(),
      api_key: provKey_.trim(),
      models,
    };
    if (provDefault.trim()) p.default_model = provDefault.trim();
    const c: Record<string, unknown> = {};
    if (nStore !== "default") c.supports_store = nStore === "true";
    if (nDev !== "default") c.supports_developer_role = nDev === "true";
    if (nEff !== "default") c.supports_reasoning_effort = nEff === "true";
    if (nTok.trim()) c.max_tokens_field = nTok.trim();
    if (Object.keys(c).length) p.compat = c;
    if (nHeaders.trim()) {
      try { p.headers = JSON.parse(nHeaders); }
      catch { toast.error("headers JSON 解析失败 — 已忽略"); }
    }
    const next = { ...cfg, providers: { ...(cfg.providers ?? {}), [k]: p } };
    if (await persist(next)) {
      toast.success("已添加 API");
      setProvOpen(false);
      setProvKey(""); setProvBase(""); setProvKey_(""); setProvModels(""); setProvDefault("");
      setNStore("default"); setNDev("default"); setNEff("default"); setNTok(""); setNHeaders("");
    }
  };

  /* ---------- provider: edit ---------- */
  const openEditProv = (key: string) => {
    const p = cfg?.providers?.[key];
    if (!p) return;
    setEditProv(key);
    setEBase(p.base_url ?? p.baseUrl ?? "");
    setEKey(p.api_key ?? p.apiKey ?? "");
    setEDefault(p.default_model ?? p.defaultModel ?? "");
    const c = p.compat ?? {};
    const tri = (v: any) => (v === true ? "true" : v === false ? "false" : "default");
    setEStore(tri(c.supports_store ?? c.supportsStore));
    setEDevRole(tri(c.supports_developer_role ?? c.supportsDeveloperRole));
    setEEffort(tri(c.supports_reasoning_effort ?? c.supportsReasoningEffort));
    setEMaxTok(c.max_tokens_field ?? c.maxTokensField ?? "");
    setEHeaders(p.headers ? JSON.stringify(p.headers) : "");
    setDelArmed(false);
  };

  const submitEditProv = async () => {
    if (!editProv || !cfg) return;
    const p = { ...cfg.providers[editProv] };
    setAliased(p, ["base_url", "baseUrl"], eBase.trim());
    setAliased(p, ["api_key", "apiKey"], eKey.trim());
    setAliased(p, ["default_model", "defaultModel"], eDefault.trim() || null);
    const compat = { ...(p.compat ?? {}) };
    const put = (k: string, ck: string, v: string) => {
      const t = triOr(v);
      if (t === undefined) { delete compat[k]; delete compat[ck]; }
      else compat[k] = t;
    };
    put("supports_store", "supportsStore", eStore);
    put("supports_developer_role", "supportsDeveloperRole", eDevRole);
    put("supports_reasoning_effort", "supportsReasoningEffort", eEffort);
    const mt = eMaxTok.trim();
    if (mt) compat.max_tokens_field = mt; else { delete compat.max_tokens_field; delete compat.maxTokensField; }
    if (Object.keys(compat).length) p.compat = compat; else delete p.compat;
    if (eHeaders.trim()) {
      try { p.headers = JSON.parse(eHeaders); }
      catch { toast.error("headers JSON 解析失败 — 已忽略"); }
    } else {
      delete p.headers;
    }
    const next = { ...cfg, providers: { ...cfg.providers, [editProv]: p } };
    if (await persist(next)) {
      toast.success("已保存");
      setEditProv(null);
    }
  };

  const deleteProv = async () => {
    if (!editProv || !cfg) return;
    const providers = { ...cfg.providers };
    delete providers[editProv];
    if (await persist({ ...cfg, providers })) {
      toast.success("已删除提供商");
      setEditProv(null);
    }
  };

  /* ---------- model: edit ---------- */
  const openEditModel = (provider: string, index: number) => {
    // index == models.length → NEW model (empty form); existing → prefilled.
    const m = normModel(cfg?.providers?.[provider]?.models?.[index]);
    setEditModel({ provider, index });
    setMId(m.id ?? "");
    setMName(m.name ?? "");
    setMCtx(m.context_window != null ? String(m.context_window) : "");
    setMReasoning(m.reasoning === true ? "true" : m.reasoning === false ? "false" : "default");
    setMInput(Array.isArray(m.input) ? m.input.join(", ") : "");
    const c = m.cost ?? {};
    setMCostIn(c.input != null ? String(c.input) : "");
    setMCostOut(c.output != null ? String(c.output) : "");
    setMCostCr(c.cache_read != null ? String(c.cache_read) : "");
    setMCostCw(c.cache_write != null ? String(c.cache_write) : "");
    const t = m.thinking_level_map ?? m.thinkingLevelMap ?? {};
    setMThink(Object.fromEntries(THINK_LEVELS.map((l) => [l, t[l] ?? ""])));
  };

  const submitEditModel = async () => {
    if (!editModel || !cfg) return;
    const p = { ...cfg.providers[editModel.provider] };
    const models = [...(p.models ?? [])];
    const m = { ...normModel(models[editModel.index]) };
    const id = mId.trim();
    if (!id) return;
    m.id = id;
    if (mName.trim()) m.name = mName.trim(); else delete m.name;
    const cw = numOr(mCtx);
    if (cw !== undefined) m.context_window = cw; else delete m.context_window;
    const r = triOr(mReasoning);
    if (r !== undefined) m.reasoning = r; else delete m.reasoning;
    const inputs = mInput.split(",").map((x) => x.trim()).filter(Boolean);
    if (inputs.length) m.input = inputs; else delete m.input;
    const cost: Record<string, number> = {};
    const ci = numOr(mCostIn), co = numOr(mCostOut), cr = numOr(mCostCr), cw2 = numOr(mCostCw);
    if (ci !== undefined) cost.input = ci;
    if (co !== undefined) cost.output = co;
    if (cr !== undefined) cost.cache_read = cr;
    if (cw2 !== undefined) cost.cache_write = cw2;
    if (Object.keys(cost).length) m.cost = cost; else delete m.cost;
    const tlm: Record<string, string> = {};
    for (const l of THINK_LEVELS) {
      const v = (mThink[l] ?? "").trim();
      if (v) tlm[l] = v;
    }
    if (Object.keys(tlm).length) m.thinking_level_map = tlm; else delete m.thinking_level_map;
    models[editModel.index] = m;
    const next = { ...cfg, providers: { ...cfg.providers, [editModel.provider]: { ...p, models } } };
    if (await persist(next)) {
      toast.success("已保存");
      setEditModel(null);
    }
  };

  const isNewModel =
    editModel !== null &&
    editModel.index >= ((cfg?.providers?.[editModel.provider]?.models ?? []).length);

  const deleteModel = async () => {
    if (!editModel || !cfg) return;
    const p = { ...cfg.providers[editModel.provider] };
    const models = [...(p.models ?? [])];
    models.splice(editModel.index, 1);
    const next = { ...cfg, providers: { ...cfg.providers, [editModel.provider]: { ...p, models } } };
    if (await persist(next)) {
      toast.success("已删除模型");
      setEditModel(null);
    }
  };

  /* ---------- render ---------- */
  const active = ov?.model;
  const provEntries = Object.entries(cfg?.providers ?? {});
  const fmt = cfgPath.endsWith(".toml") ? "TOML" : cfgPath.endsWith(".json") ? "JSON" : "";
  const TRI_OPTS = [
    { value: "default", label: "默认（省略）" },
    { value: "true", label: "是" },
    { value: "false", label: "否" },
  ];

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between gap-2">
        {cfgPath && (
          <span className="text-[11px] text-neutral-500 font-mono truncate">
            {cfgPath}{fmt ? `（${fmt}）` : ""}
          </span>
        )}
        <Button
          size="sm"
          variant="outline"
          className="h-7 text-xs gap-1 shrink-0"
          onClick={() => setProvOpen(true)}
        >
          <Plus className="h-3.5 w-3.5" /> 添加 API
        </Button>
      </div>

      {provEntries.length ? (
        provEntries.map(([pk, p]: [string, any]) => (
          <div key={pk} className="flex flex-col gap-2">
            {/* provider header */}
            <div className="flex items-center justify-between gap-2 mt-2">
              <div className="flex items-center gap-2 min-w-0">
                <span className="text-xs font-semibold text-neutral-800">{pk}</span>
                <Badge variant="secondary" className="text-[10px] px-1.5 py-0 h-4 font-mono">
                  {p.kind ?? p.type ?? p.api ?? "openai_compat"}
                </Badge>
                <span className="text-[11px] text-neutral-500 font-mono truncate">
                  {p.base_url ?? p.baseUrl ?? ""}
                </span>
              </div>
              <div className="flex items-center gap-1 shrink-0">
                <Button
                  size="sm" variant="ghost" className="h-6 w-6 p-0 text-neutral-500"
                  onClick={() => openEditProv(pk)}
                >
                  <SquarePen className="h-3.5 w-3.5" />
                </Button>
                <Button
                  size="sm" variant="ghost" className="h-6 w-6 p-0 text-neutral-500"
                  onClick={() => openEditModel(pk, (p.models ?? []).length)}
                >
                  <Plus className="h-3.5 w-3.5" />
                </Button>
              </div>
            </div>
            <KvList>
              <KvListContent>
                {(p.models ?? []).length ? (
                  (p.models ?? []).map((raw: any, i: number) => {
                    const m = normModel(raw);
                    const isActive =
                      pk === active?.active_provider && m.id === active?.active_model;
                    const thinkLvls = m.thinking_level_map
                      ? THINK_LEVELS.filter((l) => (m.thinking_level_map as Record<string, unknown>)[l] != null)
                      : [];
                    const segs = [
                      Array.isArray(m.input) && m.input.length
                        ? { icon: <Layers className="h-3 w-3" />, text: m.input.join("/") }
                        : null,
                      thinkLvls.length
                        ? { icon: <Brain className="h-3 w-3" />, text: thinkLvls.join("/") }
                        : null,
                      m.cost
                        ? { icon: <Coins className="h-3 w-3" />, text: `in ${m.cost.input ?? 0} / out ${m.cost.output ?? 0}` +
                            (m.cost.cache_read != null || m.cost.cache_write != null
                              ? ` / 缓存 ${m.cost.cache_read ?? 0}+${m.cost.cache_write ?? 0}`
                              : "") }
                        : null,
                    ].filter((x): x is NonNullable<typeof x> => x !== null);
                    return (
                      <div
                        key={m.id ?? i}
                        className="px-5 py-3 hover:bg-[color-mix(in_srgb,var(--husk-n50)_40%,transparent)] transition-colors"
                      >
                        <div className="flex items-center justify-between gap-3">
                          <div className="flex items-center gap-3 min-w-0">
                            <span className="text-neutral-500 shrink-0">
                              <Cpu className="h-4 w-4" />
                            </span>
                            <span className="text-sm text-neutral-800 truncate">
                              {m.name || m.id}
                            </span>
                          </div>
                          <div className="flex items-center gap-2 shrink-0">
                            {isActive && (
                              <Badge className="text-[10px] px-1.5 py-0 h-4">当前</Badge>
                            )}
                            {m.reasoning === true && (
                              <Badge variant="secondary" className="text-[10px] px-1.5 py-0 h-4">
                                思考
                              </Badge>
                            )}
                            {m.reasoning === false && (
                              <Badge variant="secondary" className="text-[10px] px-1.5 py-0 h-4">
                                无思考
                              </Badge>
                            )}
                            {m.context_window ? (
                              <span className="text-[11px] text-neutral-500 font-mono tabular-nums">
                                {(m.context_window / 1000).toFixed(0)}k
                              </span>
                            ) : null}
                            <Button
                              size="sm" variant="ghost"
                              className="h-6 w-6 p-0 text-neutral-500"
                              onClick={() => openEditModel(pk, i)}
                            >
                              <SquarePen className="h-3.5 w-3.5" />
                            </Button>
                          </div>
                        </div>
                        {/* Meta line spanning the full card width. */}
                        {(m.id || segs.length > 0) && (
                          <div className="mt-1.5 pl-7 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] font-mono text-neutral-500">
                            {m.id && <span>{m.id}</span>}
                            {segs.map((seg, j) => (
                              <span key={j} className="inline-flex items-center gap-1">
                                <span className="text-neutral-300">·</span>
                                {seg.icon}
                                {seg.text}
                              </span>
                            ))}
                          </div>
                        )}
                      </div>
                    );
                  })
                ) : (
                  <KvRow label="暂无模型" description="点右侧 + 添加一个模型" />
                )}
              </KvListContent>
            </KvList>
          </div>
        ))
      ) : (
        <KvList>
          <KvListContent>
            <KvRow label="暂无提供商" description="点右上角「添加 API」创建第一个" />
          </KvListContent>
        </KvList>
      )}

      {/* ======== 添加 API ======== */}
      <Dialog open={provOpen} onOpenChange={setProvOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>添加 API</DialogTitle>
          </DialogHeader>
          <div className="flex flex-col gap-3 py-1">
            <div className="grid grid-cols-2 gap-3">
              <Field label="名称（唯一标识）">
                <Input value={provKey} onChange={(e) => setProvKey(e.target.value)}
                  placeholder="openai / anthropic / …" className="h-8 text-xs font-mono" />
              </Field>
              <Field label="类型">
                <SettingSelect
                  value={provKind}
                  onChange={(v) => {
                    setProvKind(v);
                    if (!provBase.trim() && DEFAULT_BASES[v]) setProvBase(DEFAULT_BASES[v]);
                  }}
                  options={KIND_OPTIONS}
                />
              </Field>
            </div>
            <Field label="Base URL">
              <Input value={provBase} onChange={(e) => setProvBase(e.target.value)}
                placeholder="https://api.openai.com/v1" className="h-8 text-xs font-mono" />
            </Field>
            <Field label="API Key（支持 env:VAR 引用）">
              <Input value={provKey_} onChange={(e) => setProvKey_(e.target.value)}
                placeholder="sk-… 或 env:OPENAI_API_KEY" className="h-8 text-xs font-mono" />
            </Field>
            <Field label="模型列表（逗号分隔的模型 ID）">
              <Input value={provModels} onChange={(e) => setProvModels(e.target.value)}
                placeholder="gpt-5.4, gpt-5-mini" className="h-8 text-xs font-mono" />
            </Field>
            <Field label="默认模型（可选）">
              <Input value={provDefault} onChange={(e) => setProvDefault(e.target.value)}
                placeholder="默认同第一个模型" className="h-8 text-xs font-mono" />
            </Field>
            <div className="grid grid-cols-2 gap-3">
              <Field label="supports_store">
                <SettingSelect value={nStore} onChange={setNStore} options={TRI_OPTS} />
              </Field>
              <Field label="supports_developer_role">
                <SettingSelect value={nDev} onChange={setNDev} options={TRI_OPTS} />
              </Field>
              <Field label="supports_reasoning_effort">
                <SettingSelect value={nEff} onChange={setNEff} options={TRI_OPTS} />
              </Field>
              <Field label="max_tokens_field">
                <Input value={nTok} onChange={(e) => setNTok(e.target.value)}
                  placeholder="max_tokens" className="h-8 text-xs font-mono" />
              </Field>
            </div>
            <Field label="请求头 JSON（可选）">
              <Input value={nHeaders} onChange={(e) => setNHeaders(e.target.value)}
                placeholder='{"X-Title": "my-app"}' className="h-8 text-xs font-mono" />
            </Field>
          </div>
          <DialogFooter>
            <Button size="sm" className="h-8 text-xs"
              disabled={!provKey.trim() || !provBase.trim() || busy}
              onClick={() => void submitAddProvider()}>
              {busy ? "保存中…" : "添加"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* ======== 编辑提供商 ======== */}
      <Dialog open={editProv !== null} onOpenChange={(o) => !o && setEditProv(null)}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>编辑提供商 · {editProv}</DialogTitle>
          </DialogHeader>
          <div className="flex flex-col gap-3 py-1">
            <Field label="Base URL">
              <Input value={eBase} onChange={(e) => setEBase(e.target.value)} className="h-8 text-xs font-mono" />
            </Field>
            <Field label="API Key">
              <Input value={eKey} onChange={(e) => setEKey(e.target.value)} className="h-8 text-xs font-mono" />
            </Field>
            <Field label="默认模型">
              <Input value={eDefault} onChange={(e) => setEDefault(e.target.value)} className="h-8 text-xs font-mono" />
            </Field>
            <div className="grid grid-cols-2 gap-3">
              <Field label="supports_store">
                <SettingSelect value={eStore} onChange={setEStore} options={TRI_OPTS} />
              </Field>
              <Field label="supports_developer_role">
                <SettingSelect value={eDevRole} onChange={setEDevRole} options={TRI_OPTS} />
              </Field>
              <Field label="supports_reasoning_effort">
                <SettingSelect value={eEffort} onChange={setEEffort} options={TRI_OPTS} />
              </Field>
              <Field label="max_tokens_field">
                <Input value={eMaxTok} onChange={(e) => setEMaxTok(e.target.value)}
                  placeholder="max_tokens" className="h-8 text-xs font-mono" />
              </Field>
            </div>
            <Field label="请求头 JSON（可选，留空清除）">
              <Input value={eHeaders} onChange={(e) => setEHeaders(e.target.value)}
                placeholder='{"X-Title": "my-app"}' className="h-8 text-xs font-mono" />
            </Field>
          </div>
          <DialogFooter className="flex justify-between sm:justify-between">
            <Button size="sm" variant="ghost"
              className={delArmed
                ? "h-8 text-xs text-red-600 bg-red-50 hover:bg-red-100 hover:text-red-600"
                : "h-8 text-xs text-red-600 hover:text-red-600"}
              disabled={busy} onClick={armThenRun(() => void deleteProv())}>
              {delArmed ? "确认删除？再点一次" : "删除提供商"}
            </Button>
            <Button size="sm" className="h-8 text-xs" disabled={busy}
              onClick={() => void submitEditProv()}>
              {busy ? "保存中…" : "保存"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* ======== 编辑模型 ======== */}
      <Dialog open={editModel !== null} onOpenChange={(o) => !o && setEditModel(null)}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>{isNewModel ? "新建模型" : "编辑模型"} · {editModel?.provider}</DialogTitle>
          </DialogHeader>
          <div className="flex flex-col gap-3 py-1">
            <div className="grid grid-cols-2 gap-3">
              <Field label="模型 ID">
                <Input value={mId} onChange={(e) => setMId(e.target.value)} className="h-8 text-xs font-mono" />
              </Field>
              <Field label="显示名">
                <Input value={mName} onChange={(e) => setMName(e.target.value)} className="h-8 text-xs" />
              </Field>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <Field label="上下文窗口（tokens）">
                <Input value={mCtx} onChange={(e) => setMCtx(e.target.value)}
                  placeholder="262144" className="h-8 text-xs font-mono" />
              </Field>
              <Field label="推理/思考支持">
                <SettingSelect value={mReasoning} onChange={setMReasoning} options={TRI_OPTS} />
              </Field>
            </div>
            <Field label="输入模态（逗号分隔）">
              <Input value={mInput} onChange={(e) => setMInput(e.target.value)}
                placeholder="text, image" className="h-8 text-xs font-mono" />
            </Field>
            <div className="grid grid-cols-2 gap-3">
              <Field label="成本 input">
                <Input value={mCostIn} onChange={(e) => setMCostIn(e.target.value)} className="h-8 text-xs font-mono" />
              </Field>
              <Field label="成本 output">
                <Input value={mCostOut} onChange={(e) => setMCostOut(e.target.value)} className="h-8 text-xs font-mono" />
              </Field>
              <Field label="成本 cache_read">
                <Input value={mCostCr} onChange={(e) => setMCostCr(e.target.value)} className="h-8 text-xs font-mono" />
              </Field>
              <Field label="成本 cache_write">
                <Input value={mCostCw} onChange={(e) => setMCostCw(e.target.value)} className="h-8 text-xs font-mono" />
              </Field>
            </div>
            <div className="grid grid-cols-2 gap-3">
              {THINK_LEVELS.map((l) => (
                <Field key={l} label={`思考 ${l}`}>
                  <Input value={mThink[l] ?? ""} onChange={(e) => setMThink({ ...mThink, [l]: e.target.value })}
                    placeholder={l} className="h-8 text-xs font-mono" />
                </Field>
              ))}
            </div>
            <p className="text-[11px] text-neutral-500">
              思考等级映射：留空 = 不映射（按提供商默认传参）
            </p>
          </div>
          <DialogFooter className={isNewModel ? "flex justify-end" : "flex justify-between sm:justify-between"}>
            {!isNewModel && (
              <Button size="sm" variant="ghost"
                className={delArmed
                  ? "h-8 text-xs text-red-600 bg-red-50 hover:bg-red-100 hover:text-red-600"
                  : "h-8 text-xs text-red-600 hover:text-red-600"}
                disabled={busy} onClick={armThenRun(() => void deleteModel())}>
                {delArmed ? "确认删除？再点一次" : "删除模型"}
              </Button>
            )}
            <Button size="sm" className="h-8 text-xs" disabled={busy || !mId.trim()}
              onClick={() => void submitEditModel()}>
              {busy ? "保存中…" : isNewModel ? "添加" : "保存"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

/* ============================ 技能 ============================ */

function SkillsPane({ ov, reload }: { ov: AgentOverview | null; reload: () => void }) {
  const [scope, setScope] = useState<"all" | "workspace" | "global">("all");
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [desc, setDesc] = useState("");
  const [target, setTarget] = useState<"workspace" | "global">("workspace");
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);

  const filtered = (ov?.skills ?? []).filter(
    (s) => scope === "all" || (scope === "global" ? s.global : !s.global),
  );

  const submit = async () => {
    if (!name.trim()) return;
    setBusy(true);
    try {
      await createSkill({
        name: name.trim(),
        description: desc.trim(),
        scope: target,
        body: body.trim() || `# ${name.trim()}\n`,
      });
      toast.success("已创建技能");
      setOpen(false);
      setName(""); setDesc(""); setBody("");
      reload();
    } catch (e) {
      toast.error("创建失败", { description: String(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-1">
          {(["all", "workspace", "global"] as const).map((v) => (
            <button
              key={v}
              onClick={() => setScope(v)}
              className={cn(
                "h-7 px-2.5 rounded-md text-xs transition-colors cursor-pointer",
                scope === v
                  ? "bg-[color-mix(in_srgb,var(--husk-black)_6%,transparent)] text-neutral-900"
                  : "text-neutral-500 hover:text-neutral-900",
              )}
            >
              {v === "all" ? "全部" : v === "workspace" ? "工作区" : "全局"}
            </button>
          ))}
        </div>
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
          {filtered.length ? (
            filtered.map((s) => (
              <KvRow
                key={s.path}
                label={s.name}
                description={
                  s.description ? (
                    <span className="line-clamp-3">{s.description}</span>
                  ) : undefined
                }
                icon={<Sparkles className="h-4 w-4" />}
              >
                <div className="flex items-center gap-2">
                  {s.global && (
                    <Badge variant="secondary" className="text-[10px] px-1.5 py-0 h-4">
                      全局
                    </Badge>
                  )}
                  <span className="text-[11px] text-neutral-500 font-mono max-w-[220px] truncate">
                    {s.path}
                  </span>
                </div>
              </KvRow>
            ))
          ) : (
            <KvRow
              label={
                scope === "workspace"
                  ? "工作区暂无技能"
                  : scope === "global"
                    ? "全局暂无技能"
                    : "暂无技能"
              }
              description="点右上角「添加」创建一个，或切换其他范围查看"
            />
          )}
        </KvListContent>
      </KvList>

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>添加技能</DialogTitle>
          </DialogHeader>
          <div className="flex flex-col gap-3 py-1">
            <div className="grid grid-cols-2 gap-3">
              <Field label="名称（小写 slug）">
                <Input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="my-skill"
                  className="h-8 text-xs font-mono"
                />
              </Field>
              <Field label="位置">
                <SettingSelect
                  value={target}
                  onChange={(v) => setTarget(v as "workspace" | "global")}
                  options={[
                    { value: "workspace", label: "工作区" },
                    { value: "global", label: "全局" },
                  ]}
                />
              </Field>
            </div>
            <Field label="描述">
              <Input
                value={desc}
                onChange={(e) => setDesc(e.target.value)}
                placeholder="什么时候用这个技能"
                className="h-8 text-xs"
              />
            </Field>
            <Field label="内容（SKILL.md 正文）">
              <Textarea
                value={body}
                onChange={(e) => setBody(e.target.value)}
                rows={6}
                placeholder="# 技能说明与步骤…"
                className="font-mono text-[12px] leading-relaxed"
              />
            </Field>
          </div>
          <DialogFooter>
            <Button
              size="sm"
              className="h-8 text-xs"
              disabled={!name.trim() || busy}
              onClick={() => void submit()}
            >
              {busy ? "创建中…" : "创建"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

/* ============================ MCP ============================ */

function McpPane({ ov, reload }: { ov: AgentOverview | null; reload: () => void }) {
  const [open, setOpen] = useState(false);
  const [id, setId] = useState("");
  const [name, setName] = useState("");
  const [transport, setTransport] = useState<"stdio" | "http">("stdio");
  const [command, setCommand] = useState("");
  const [args, setArgs] = useState("");
  const [url, setUrl] = useState("");
  const [headers, setHeaders] = useState("");
  const [busy, setBusy] = useState(false);

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
      toast.success("已添加 MCP 插件");
      setOpen(false);
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
              <KvRow
                key={p.id}
                label={`${p.name} v${p.version}`}
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
                  <span className="text-[11px] text-neutral-500 tabular-nums">
                    {p.tools} 工具
                  </span>
                </div>
              </KvRow>
            ))
          ) : (
            <KvRow label="暂无插件" description="plugins 目录下未发现 manifest.json" />
          )}
        </KvListContent>
      </KvList>

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>添加 MCP 插件</DialogTitle>
          </DialogHeader>
          <div className="flex flex-col gap-3 py-1">
            <div className="grid grid-cols-2 gap-3">
              <Field label="ID（小写 slug）">
                <Input
                  value={id}
                  onChange={(e) => setId(e.target.value)}
                  placeholder="my-server"
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
          <DialogFooter>
            <Button
              size="sm"
              className="h-8 text-xs"
              disabled={
                !id.trim() ||
                busy ||
                (transport === "stdio" ? !command.trim() : !url.trim())
              }
              onClick={() => void submit()}
            >
              {busy ? "保存中…" : "添加"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

/* ============================ SubAgent ============================ */

function SubagentPane({ ov, reload }: { ov: AgentOverview | null; reload: () => void }) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [desc, setDesc] = useState("");
  const [scope, setScope] = useState<"workspace" | "global">("workspace");
  const [prompt, setPrompt] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (!name.trim() || !prompt.trim()) return;
    setBusy(true);
    try {
      await createSubagent({
        name: name.trim(),
        description: desc.trim(),
        scope,
        prompt: prompt.trim(),
      });
      toast.success("已创建子代理");
      setOpen(false);
      setName(""); setDesc(""); setPrompt("");
      reload();
    } catch (e) {
      toast.error("创建失败", { description: String(e) });
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
          {ov?.subagents.map((a) => (
            <KvRow
              key={a.name}
              label={a.name}
              description={a.description || undefined}
              icon={<Bot className="h-4 w-4" />}
            >
              <div className="flex items-center gap-2">
                {a.builtin ? (
                  <Badge variant="secondary" className="text-[10px] px-1.5 py-0 h-4">
                    内置
                  </Badge>
                ) : (
                  <>
                    <Badge variant="secondary" className="text-[10px] px-1.5 py-0 h-4">
                      {a.global ? "全局" : "工作区"}
                    </Badge>
                    {a.path && (
                      <span className="text-[11px] text-neutral-500 font-mono max-w-[200px] truncate">
                        {a.path}
                      </span>
                    )}
                  </>
                )}
              </div>
            </KvRow>
          ))}
        </KvListContent>
      </KvList>

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>添加子代理</DialogTitle>
          </DialogHeader>
          <div className="flex flex-col gap-3 py-1">
            <div className="grid grid-cols-2 gap-3">
              <Field label="名称（小写 slug）">
                <Input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="reviewer"
                  className="h-8 text-xs font-mono"
                />
              </Field>
              <Field label="位置">
                <SettingSelect
                  value={scope}
                  onChange={(v) => setScope(v as "workspace" | "global")}
                  options={[
                    { value: "workspace", label: "工作区" },
                    { value: "global", label: "全局" },
                  ]}
                />
              </Field>
            </div>
            <Field label="描述">
              <Input
                value={desc}
                onChange={(e) => setDesc(e.target.value)}
                placeholder="这个子代理擅长什么"
                className="h-8 text-xs"
              />
            </Field>
            <Field label="系统提示词（子代理的行为指令）">
              <Textarea
                value={prompt}
                onChange={(e) => setPrompt(e.target.value)}
                rows={7}
                placeholder="你是一个代码审查专家，专注于…"
                className="font-mono text-[12px] leading-relaxed"
              />
            </Field>
          </div>
          <DialogFooter>
            <Button
              size="sm"
              className="h-8 text-xs"
              disabled={!name.trim() || !prompt.trim() || busy}
              onClick={() => void submit()}
            >
              {busy ? "创建中…" : "创建"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

/* ============================ 入口 ============================ */

export function AgentSettings({ tab }: { tab: AgentTab }) {
  const [ov, setOv] = useState<AgentOverview | null>(null);

  const reload = () => {
    void getAgentOverview()
      .then(setOv)
      .catch(() => {});
  };
  useEffect(reload, []);

  return (
    <div className="flex flex-col gap-8 w-full">
      {tab === "instructions" && <InstructionsPane />}
      {tab === "model" && <ModelPane ov={ov} reload={reload} />}
      {tab === "skills" && <SkillsPane ov={ov} reload={reload} />}
      {tab === "mcp" && <McpPane ov={ov} reload={reload} />}
      {tab === "subagent" && <SubagentPane ov={ov} reload={reload} />}
    </div>
  );
}
