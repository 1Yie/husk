// 模型 pane — providers and their models (add / edit / delete).

import { SkeletonKvList } from "@/components/ui/skeleton";
import { toast } from "sonner";
import { useEffect, useState } from "react";
import {
  ArrowDown,
  ArrowUp,
  Brain,
  Coins,
  Cpu,
  Layers,
  Plus,
  SquarePen,
  X,
} from "@keyline-icons/react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { TooltipSimple } from "@/components/ui/tooltip";
import { Textarea } from "@/components/ui/textarea";
import { KvList, KvListContent, KvRow } from "@/components/ui/kv-list";
import { SettingSelect } from "@/features/settings/components/index";
import { ArmedDeleteButton } from "@/features/settings/components/armed-delete";
import { FormDialog } from "@/features/settings/components/form-dialog";
import { getAppConfig, saveAppConfig, type AgentOverview } from "@/lib/agent-ipc/sessions";
import { Field } from "@/features/settings/pages/agent/shared/index";

/* ============================ 模型 ============================ */

/** Provider kinds the config schema accepts — labeled by their real API
 * names (openai_completions IS the Chat Completions API). */
const KIND_OPTIONS = [
  { value: "openai_completions", label: "OpenAI Chat Completions" },
  { value: "openai_responses", label: "OpenAI Responses" },
  { value: "anthropic", label: "Anthropic (Claude)" },
  { value: "gemini", label: "Google Gemini" },
];

const DEFAULT_BASES: Record<string, string> = {
  openai_completions: "https://api.openai.com/v1",
  openai_responses: "https://api.openai.com/v1",
  anthropic: "https://api.anthropic.com",
  // rig appends `/v1beta/models/...` itself; a `/v1beta` suffix here doubles it.
  gemini: "https://generativelanguage.googleapis.com",
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

/** Arm a destructive button for a few seconds.
 *
 *  The label flips to a confirm on the first click; if the second one never
 *  comes the button flips back on its own. Left armed, the row reads as a
 *  broken button and the next click deletes without asking.
 */

export function ModelPane({ ov, reload }: { ov: AgentOverview | null; reload: () => void }) {
  const [cfg, setCfg] = useState<Record<string, any> | null>(null);
  const [cfgPath, setCfgPath] = useState("");
  // Set when the file exists but does not parse. Editing is disabled then:
  // the in-memory copy is empty, and saving it would replace the file.
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // ---- add-provider dialog ----
  const [provOpen, setProvOpen] = useState(false);
  const [provKey, setProvKey] = useState("");
  const [provKind, setProvKind] = useState("openai_completions");
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
  const [eKind, setEKind] = useState("openai_completions");
  const [eName, setEName] = useState("");
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
        setLoadError(ac.load_error ?? null);
      })
      .catch(() => {});
  };
  useEffect(loadCfg, []);

  const persist = async (next: Record<string, any>) => {
    if (loadError) {
      toast.error("配置文件无法解析，已禁止保存", { description: loadError });
      return false;
    }
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
    setEName(key);
    setEBase(p.base_url ?? p.baseUrl ?? "");
    setEKind(p.kind ?? p.type ?? p.api ?? "openai_completions");
    setEKey(p.api_key ?? p.apiKey ?? "");
    setEDefault(p.default_model ?? p.defaultModel ?? "");
    const c = p.compat ?? {};
    const tri = (v: any) => (v === true ? "true" : v === false ? "false" : "default");
    setEStore(tri(c.supports_store ?? c.supportsStore));
    setEDevRole(tri(c.supports_developer_role ?? c.supportsDeveloperRole));
    setEEffort(tri(c.supports_reasoning_effort ?? c.supportsReasoningEffort));
    setEMaxTok(c.max_tokens_field ?? c.maxTokensField ?? "");
    setEHeaders(p.headers ? JSON.stringify(p.headers) : "");
  };

  const submitEditProv = async () => {
    if (!editProv || !cfg) return;
    const oldKey = editProv;
    const newKey = eName.trim().toLowerCase();
    if (!newKey) { toast.error("名称不能为空"); return; }
    // Keys are case-sensitive in config.toml but the add flow lowercases them,
    // so compare normalized against every other key — not just an exact hit.
    const clash = Object.keys(cfg.providers ?? {}).some(
      (k) => k !== oldKey && k.toLowerCase() === newKey,
    );
    if (clash) { toast.error("该名称已存在"); return; }
    const p = { ...cfg.providers[oldKey] };
    setAliased(p, ["base_url", "baseUrl"], eBase.trim());
    p.kind = eKind;
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
    // Rename: rebuild the map in place so the provider keeps its position,
    // then rewrite every key reference (active provider, fallback chain).
    const providers: Record<string, any> = {};
    for (const [k, v] of Object.entries(cfg.providers ?? {})) {
      providers[k === oldKey ? newKey : k] = k === oldKey ? p : v;
    }
    const renameRef = (k: string) => (k === oldKey ? newKey : k);
    const next: Record<string, any> = { ...cfg, providers };
    if (typeof cfg.active_provider === "string") next.active_provider = renameRef(cfg.active_provider);
    if (typeof cfg.activeProvider === "string") next.activeProvider = renameRef(cfg.activeProvider);
    const chain = cfg.fallback_chain ?? cfg.fallbackChain;
    if (Array.isArray(chain)) {
      const renamed = chain.map(renameRef);
      if (cfg.fallback_chain !== undefined) next.fallback_chain = renamed;
      if (cfg.fallbackChain !== undefined) next.fallbackChain = renamed;
    }
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

  /* ---------- fallback chain ---------- */
  // Global, ordered degrade chain (config `fallback_chain`). Stored as plain
  // names. The picker offers configured providers not yet in the chain. The
  // backend additionally drops whichever entry equals the session's own
  // primary, so a listed primary is harmless: it is skipped at runtime.
  const chain: string[] = Array.isArray(cfg?.fallback_chain)
    ? cfg!.fallback_chain
    : Array.isArray(cfg?.fallbackChain)
      ? cfg!.fallbackChain
      : [];
  const chainKey = cfg?.fallback_chain !== undefined || cfg?.fallbackChain === undefined
    ? "fallback_chain"
    : "fallbackChain";
  const saveChain = (next: string[]) => {
    if (!cfg) return;
    void persist({ ...cfg, [chainKey]: next });
  };
  const chainCandidates = Object.keys(cfg?.providers ?? {}).filter((k) => !chain.includes(k));

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
      {loadError && (
        <div className="rounded-md border border-red-200 bg-red-50 px-3 py-2 text-[11px] text-red-700">
          <div className="font-medium">配置文件无法解析，已禁止编辑以免覆盖原文件</div>
          <div className="font-mono break-all">{loadError}</div>
        </div>
      )}
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
          disabled={!!loadError}
          onClick={() => setProvOpen(true)}
        >
          <Plus className="h-3.5 w-3.5" /> 添加 API
        </Button>
      </div>

      {cfg && (
        <div className="flex flex-col gap-2 rounded-md border border-neutral-200 px-3 py-2">
          <div className="flex items-center justify-between gap-2">
            <span className="text-xs font-semibold text-neutral-800">降级链</span>
            <span className="text-[11px] text-neutral-500">
              主提供商耗尽重试后，按顺序切换到下列备用
            </span>
          </div>
          {chain.length ? (
            <ol className="flex flex-col gap-1">
              {chain.map((name, i) => (
                <li key={name} className="flex items-center justify-between gap-2 text-xs">
                  <span className="font-mono text-neutral-800">
                    {i + 1}. {name}
                  </span>
                  <div className="flex items-center gap-1">
                    <Button size="sm" variant="ghost" className="h-6 w-6 p-0 text-neutral-500"
                      disabled={i === 0 || busy}
                      onClick={() => {
                        const next = [...chain];
                        [next[i - 1], next[i]] = [next[i], next[i - 1]];
                        saveChain(next);
                      }}>
                      <ArrowUp className="h-3.5 w-3.5" />
                    </Button>
                    <Button size="sm" variant="ghost" className="h-6 w-6 p-0 text-neutral-500"
                      disabled={i === chain.length - 1 || busy}
                      onClick={() => {
                        const next = [...chain];
                        [next[i], next[i + 1]] = [next[i + 1], next[i]];
                        saveChain(next);
                      }}>
                      <ArrowDown className="h-3.5 w-3.5" />
                    </Button>
                    <Button size="sm" variant="ghost" className="h-6 w-6 p-0 text-neutral-500"
                      disabled={busy}
                      onClick={() => saveChain(chain.filter((n) => n !== name))}>
                      <X className="h-3.5 w-3.5" />
                    </Button>
                  </div>
                </li>
              ))}
            </ol>
          ) : (
            <span className="text-[11px] text-neutral-500">未设置，主提供商失败时不会切换</span>
          )}
          {chainCandidates.length > 0 && (
            <SettingSelect
              value=""
              placeholder="添加备用提供商…"
              onChange={(v) => v && saveChain([...chain, v])}
              options={chainCandidates.map((k) => ({ value: k, label: k }))}
            />
          )}
        </div>
      )}
      {!cfg ? (
        // Only the provider list waits on the config; the header above is
        // usable straight away.
        <SkeletonKvList rows={2} />
      ) : provEntries.length ? (
        provEntries.map(([pk, p]: [string, any]) => (
          <div key={pk} className="flex flex-col gap-2">
            {/* provider header */}
            <div className="flex items-center justify-between gap-2 mt-2">
              <div className="flex items-center gap-2 min-w-0">
                <span className="text-xs font-semibold text-neutral-800">{pk}</span>
                <Badge variant="secondary" className="text-[10px] px-1.5 py-0 h-4 font-mono">
                  {p.kind ?? p.type ?? p.api ?? "openai_completions"}
                </Badge>
                <span className="text-[11px] text-neutral-500 font-mono truncate">
                  {p.base_url ?? p.baseUrl ?? ""}
                </span>
              </div>
              <div className="flex items-center gap-1 shrink-0">
                <TooltipSimple content="编辑提供商" side="top">
                  <Button
                    size="sm" variant="ghost" className="h-6 w-6 p-0 text-neutral-500"
                    onClick={() => openEditProv(pk)}
                  >
                    <SquarePen className="h-3.5 w-3.5" />
                  </Button>
                </TooltipSimple>
                <TooltipSimple content="新建模型" side="top">
                  <Button
                    size="sm" variant="ghost" className="h-6 w-6 p-0 text-neutral-500"
                    onClick={() => openEditModel(pk, (p.models ?? []).length)}
                  >
                    <Plus className="h-3.5 w-3.5" />
                  </Button>
                </TooltipSimple>
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
                            <TooltipSimple content="编辑模型" side="top">
                              <Button
                                size="sm" variant="ghost"
                                className="h-6 w-6 p-0 text-neutral-500"
                                onClick={() => openEditModel(pk, i)}
                              >
                                <SquarePen className="h-3.5 w-3.5" />
                              </Button>
                            </TooltipSimple>
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
      <FormDialog
        open={provOpen}
        onOpenChange={setProvOpen}
        title="添加 API"
        submitLabel="添加"
        busy={busy}
        disabled={!provKey.trim() || !provBase.trim()}
        onSubmit={() => void submitAddProvider()}
      >
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
              <Input type="password" value={provKey_} onChange={(e) => setProvKey_(e.target.value)}
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
              <Textarea value={nHeaders} onChange={(e) => setNHeaders(e.target.value)}
                rows={3} placeholder={'{\n  "X-Title": "my-app"\n}'} className="text-xs font-mono resize-y max-h-40" />
            </Field>
          </div>
      </FormDialog>

      {/* ======== 编辑提供商 ======== */}
      <FormDialog
        open={editProv !== null}
        onOpenChange={(o: boolean) => !o && setEditProv(null)}
        title="编辑提供商"
        submitLabel="保存"
        busy={busy}
        onSubmit={() => void submitEditProv()}
        secondary={
          <ArmedDeleteButton
            variant="text"
            label="删除提供商"
            busy={busy}
            onConfirm={() => void deleteProv()}
          />
        }
      >
          <div className="flex flex-col gap-3 py-1">
            <div className="grid grid-cols-2 gap-3">
              <Field label="名称（唯一标识）">
                <Input value={eName} onChange={(e) => setEName(e.target.value)}
                  placeholder="openai / anthropic / …" className="h-8 text-xs font-mono" />
              </Field>
              <Field label="类型">
                <SettingSelect value={eKind} onChange={setEKind} options={KIND_OPTIONS} />
              </Field>
            </div>
            <Field label="Base URL">
              <Input value={eBase} onChange={(e) => setEBase(e.target.value)} className="h-8 text-xs font-mono" />
            </Field>
            <Field label="API Key">
              {/* Stored plaintext keys arrive masked; submitting the mask keeps the stored key. */}
              <Input type="password" value={eKey} onChange={(e) => setEKey(e.target.value)} className="h-8 text-xs font-mono" />
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
              <Textarea value={eHeaders} onChange={(e) => setEHeaders(e.target.value)}
                rows={3} placeholder={'{\n  "X-Title": "my-app"\n}'} className="text-xs font-mono resize-y max-h-40" />
            </Field>
          </div>
      </FormDialog>

      {/* ======== 编辑模型 ======== */}
      <FormDialog
        open={editModel !== null}
        onOpenChange={(o: boolean) => !o && setEditModel(null)}
        title={isNewModel ? "新建模型" : "编辑模型"}
        submitLabel={isNewModel ? "添加" : "保存"}
        busy={busy}
        disabled={!mId.trim()}
        onSubmit={() => void submitEditModel()}
        secondary={
          isNewModel ? undefined : (
            <ArmedDeleteButton
              variant="text"
              label="删除模型"
              busy={busy}
              onConfirm={() => void deleteModel()}
            />
          )
        }
      >
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
      </FormDialog>
    </div>
  );
}
