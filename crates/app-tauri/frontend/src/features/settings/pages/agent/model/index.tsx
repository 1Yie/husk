// 模型 pane — providers and their models (add / edit / delete).

import { SkeletonKvList } from "@/components/ui/skeleton";
import { toast } from "sonner";
import { useEffect, useState } from "react";
import { ChevronRight, Cpu, Plus, SquarePen } from "@keyline-icons/react";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Input } from "@/components/ui/input";
import { TooltipSimple } from "@/components/ui/tooltip";
import { Textarea } from "@/components/ui/textarea";
import { KvList, KvListContent } from "@/components/ui/kv-list";
import { SettingSelect } from "@/features/settings/components/index";
import { ArmedDeleteButton } from "@/features/settings/components/armed-delete";
import { FormDialog } from "@/features/settings/components/form-dialog";
import { getAppConfig, saveAppConfig, type AgentOverview } from "@/lib/agent-ipc/sessions";
import { EmptyState, Field, ItemIcon, Tag } from "@/features/settings/pages/agent/shared/index";
import { ApiKeyInput } from "./api-key-input";
import { FallbackChain } from "./fallback-chain";

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
import { THINKING_LEVELS } from "@/lib/thinking";

const triOr = (v: string) =>
  v === "true" ? true : v === "false" ? false : undefined;

/** A titled group inside an editor dialog — keeps a long form scannable. */
function Group({ title, hint, children }: { title: string; hint?: string; children: React.ReactNode }) {
  return (
    <section className="flex flex-col gap-2.5">
      <div className="flex items-baseline justify-between gap-2">
        <span className="text-[11px] font-semibold uppercase tracking-wide text-neutral-500">{title}</span>
        {hint && <span className="text-[11px] text-neutral-400">{hint}</span>}
      </div>
      {children}
    </section>
  );
}

/** Rarely-touched options, folded away until asked for. */
function Advanced({ label, children }: { label: string; children: React.ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <CollapsibleTrigger className="flex w-full cursor-pointer items-center gap-1 text-[11px] font-semibold uppercase tracking-wide text-neutral-500 hover:text-neutral-800">
        <ChevronRight className={`h-3 w-3 transition-transform ${open ? "rotate-90" : ""}`} />
        {label}
      </CollapsibleTrigger>
      {/* Keep the 3px focus rings inside the animation's clipping boundary.
          Negative margins preserve alignment with the other form fields. */}
      <CollapsibleContent className="-mx-1">
        <div className="flex flex-col gap-3 px-1 pb-1 pt-3">{children}</div>
      </CollapsibleContent>
    </Collapsible>
  );
}

/** Scrolls inside the dialog instead of growing it past the window. */
const FORM = "flex max-h-[60vh] flex-col gap-5 overflow-y-auto p-1";

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
    const k = provKey.trim();
    // Names keep the case they were typed in; only the clash check ignores it,
    // so `DeepSeek` and `deepseek` cannot both exist.
    if (Object.keys(cfg.providers ?? {}).some((x) => x.toLowerCase() === k.toLowerCase())) {
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
    const newKey = eName.trim();
    if (!newKey) { toast.error("名称不能为空"); return; }
    // Case-only renames (`deepseek` → `DeepSeek`) are allowed; any OTHER key
    // that differs only by case is a clash.
    const clash = Object.keys(cfg.providers ?? {}).some(
      (k) => k !== oldKey && k.toLowerCase() === newKey.toLowerCase(),
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
    // The api_key comes back masked; the backend restores it by id, so a
    // renamed provider has to name the id its key is stored under.
    if (newKey !== oldKey) p.renamed_from = oldKey;
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
      // Re-read so the one-shot `renamed_from` marker never rides a later save.
      if (newKey !== oldKey) loadCfg();
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
    setMThink(Object.fromEntries(THINKING_LEVELS.map((l) => [l.value, t[l.value] ?? ""])));
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
    for (const l of THINKING_LEVELS) {
      const v = (mThink[l.value] ?? "").trim();
      if (v) tlm[l.value] = v;
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
    if (!cfg || busy || loadError) return;
    void persist({ ...cfg, [chainKey]: next });
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
    <div className="flex flex-col gap-4">
      {loadError && (
        <div className="rounded-xl border border-red-200 bg-red-50 px-4 py-3 text-xs text-red-700">
          <div className="font-medium">配置文件无法解析，已禁止编辑以免覆盖原文件</div>
          <div className="mt-1 font-mono text-[11px] break-all">{loadError}</div>
        </div>
      )}
      <div className="mb-4">
        <FallbackChain
          chain={chain}
          providers={cfg?.providers ?? {}}
          loading={!cfg}
          busy={busy}
          disabled={!!loadError}
          onChange={saveChain}
          onAddProvider={() => setProvOpen(true)}
        />
      </div>
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-0.5">
          <span className="text-sm font-semibold text-neutral-900">模型提供商</span>
          {cfgPath && (
            <span className="truncate text-[11px] text-neutral-500" title={cfgPath}>
              配置文件 · {cfgPath}{fmt ? `（${fmt}）` : ""}
            </span>
          )}
        </div>
        <Button
          size="sm"
          variant="outline"
          className="h-7 shrink-0 gap-1 text-xs"
          disabled={!!loadError}
          onClick={() => setProvOpen(true)}
        >
          <Plus className="h-3.5 w-3.5" /> 添加 API
        </Button>
      </div>

      {!cfg ? (
        // Only the provider list waits on the config; the header above is
        // usable straight away.
        <SkeletonKvList rows={2} />
      ) : provEntries.length ? (
        provEntries.map(([pk, p]: [string, any]) => {
          const models: any[] = p.models ?? [];
          return (
            <KvList key={pk}>
              {/* provider header — the card owns its title, URL and actions */}
              <div className="flex items-center gap-3 border-b border-neutral-100 px-5 py-3.5 dark:border-white/8">
                <ItemIcon>
                  <Cpu className="h-4 w-4" />
                </ItemIcon>
                <div className="flex min-w-0 flex-1 flex-col gap-1">
                  <div className="flex min-w-0 items-center gap-1.5">
                    <span className="truncate text-sm font-semibold text-neutral-900">{pk}</span>
                    <Tag mono>{p.kind ?? p.type ?? p.api ?? "openai_completions"}</Tag>
                    {pk === active?.active_provider && <Tag tone="success">使用中</Tag>}
                    <span className="shrink-0 text-[11px] tabular-nums text-neutral-400">
                      {models.length} 个模型
                    </span>
                  </div>
                  <span
                    className="w-fit max-w-full truncate font-mono text-[11px] text-neutral-400"
                    title={p.base_url ?? p.baseUrl ?? ""}
                  >
                    {p.base_url ?? p.baseUrl ?? "未设置 Base URL"}
                  </span>
                </div>
                <TooltipSimple content="编辑提供商" side="top">
                  <Button
                    size="sm" variant="ghost" aria-label="编辑提供商"
                    className="h-7 w-7 shrink-0 p-0 text-neutral-500"
                    onClick={() => openEditProv(pk)}
                  >
                    <SquarePen className="h-3.5 w-3.5" />
                  </Button>
                </TooltipSimple>
              </div>
              <KvListContent className="dark:divide-[var(--husk-border)]">
                {models.map((raw: any, i: number) => {
                  const m = normModel(raw);
                  const isActive = pk === active?.active_provider && m.id === active?.active_model;
                  const thinkLvls = m.thinking_level_map
                    ? THINKING_LEVELS.filter((l) => (m.thinking_level_map as Record<string, unknown>)[l.value] != null)
                    : [];
                  const meta = [
                    m.name && m.name !== m.id ? m.id : null,
                    m.context_window ? `${(m.context_window / 1000).toFixed(0)}k 上下文` : null,
                    Array.isArray(m.input) && m.input.length ? m.input.join("/") : null,
                    m.reasoning === true
                      ? thinkLvls.length ? `思考 ${thinkLvls.map((l) => l.label).join("/")}` : "支持思考"
                      : m.reasoning === false ? "无思考" : null,
                    m.cost ? `in ${m.cost.input ?? 0} · out ${m.cost.output ?? 0}` : null,
                  ].filter((x): x is string => !!x);
                  return (
                    <div
                      key={m.id ?? i}
                      role="button"
                      tabIndex={0}
                      onClick={() => openEditModel(pk, i)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          openEditModel(pk, i);
                        }
                      }}
                      className="group flex cursor-pointer items-center justify-between gap-3 py-2.5 pl-16 pr-5 transition-colors hover:bg-[color-mix(in_srgb,var(--husk-n50)_40%,transparent)] focus-visible:bg-[color-mix(in_srgb,var(--husk-n50)_40%,transparent)] focus-visible:outline-none"
                    >
                      <div className="flex min-w-0 flex-col gap-0.5">
                        <div className="flex min-w-0 items-center gap-1.5">
                          <span className="truncate text-sm text-neutral-800">{m.name || m.id}</span>
                          {isActive && <Tag tone="success">当前</Tag>}
                        </div>
                        {meta.length > 0 && (
                          <span className="truncate text-[11px] text-neutral-500">
                            {meta.map((t, j) => (
                              <span key={j} className={j === 0 && m.name && m.name !== m.id ? "font-mono" : undefined}>
                                {j > 0 && <span className="px-1.5 text-neutral-300">·</span>}
                                {t}
                              </span>
                            ))}
                          </span>
                        )}
                      </div>
                      <SquarePen className="h-3.5 w-3.5 shrink-0 text-neutral-400 opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100" />
                    </div>
                  );
                })}
                {models.length === 0 && (
                  <div className="py-3 pl-16 pr-5 text-xs italic text-neutral-400">尚未添加模型</div>
                )}
                <button
                  type="button"
                  onClick={() => openEditModel(pk, models.length)}
                  className="flex w-full cursor-pointer items-center gap-2 py-2.5 pl-16 pr-5 text-xs text-neutral-500 transition-colors hover:bg-[color-mix(in_srgb,var(--husk-n50)_40%,transparent)] hover:text-neutral-800"
                >
                  <Plus className="h-3.5 w-3.5" /> 添加模型
                </button>
              </KvListContent>
            </KvList>
          );
        })
      ) : (
        <KvList>
          <EmptyState
            icon={<Cpu className="h-4 w-4" />}
            title="暂无模型提供商"
            description="接入 OpenAI、Anthropic、Gemini 或兼容端点后即可选择模型"
            action={
              <Button
                size="sm"
                variant="outline"
                className="h-7 gap-1 text-xs"
                disabled={!!loadError}
                onClick={() => setProvOpen(true)}
              >
                <Plus className="h-3.5 w-3.5" /> 添加 API
              </Button>
            }
          />
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
          <div className={FORM}>
            <Group title="连接">
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
                <ApiKeyInput key={String(provOpen)} value={provKey_} onChange={setProvKey_} />
              </Field>
            </Group>
            <Group title="模型">
              <Field label="模型列表（逗号分隔的模型 ID）">
                <Input value={provModels} onChange={(e) => setProvModels(e.target.value)}
                  placeholder="gpt-5.4, gpt-5-mini" className="h-8 text-xs font-mono" />
              </Field>
              <Field label="默认模型（可选）">
                <Input value={provDefault} onChange={(e) => setProvDefault(e.target.value)}
                  placeholder="默认同第一个模型" className="h-8 text-xs font-mono" />
              </Field>
            </Group>
            <Advanced label="兼容性与请求头">
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
            </Advanced>
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
          <div className={FORM}>
            <Group title="连接">
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
                <ApiKeyInput key={editProv} value={eKey} onChange={setEKey} provider={editProv ?? undefined} />
              </Field>
            </Group>
            <Group title="模型">
              <Field label="默认模型">
                <Input value={eDefault} onChange={(e) => setEDefault(e.target.value)} className="h-8 text-xs font-mono" />
              </Field>
            </Group>
            <Advanced label="兼容性与请求头">
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
            </Advanced>
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
          <div className={FORM}>
            <Group title="标识">
              <div className="grid grid-cols-2 gap-3">
                <Field label="模型 ID">
                  <Input value={mId} onChange={(e) => setMId(e.target.value)} className="h-8 text-xs font-mono" />
                </Field>
                <Field label="显示名">
                  <Input value={mName} onChange={(e) => setMName(e.target.value)} className="h-8 text-xs" />
                </Field>
              </div>
            </Group>
            <Group title="能力">
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
            </Group>
            <Group title="价格">
              <div className="grid grid-cols-2 gap-3">
                <Field label="输入">
                  <Input value={mCostIn} onChange={(e) => setMCostIn(e.target.value)} className="h-8 text-xs font-mono" />
                </Field>
                <Field label="输出">
                  <Input value={mCostOut} onChange={(e) => setMCostOut(e.target.value)} className="h-8 text-xs font-mono" />
                </Field>
                <Field label="缓存读取">
                  <Input value={mCostCr} onChange={(e) => setMCostCr(e.target.value)} className="h-8 text-xs font-mono" />
                </Field>
                <Field label="缓存写入">
                  <Input value={mCostCw} onChange={(e) => setMCostCw(e.target.value)} className="h-8 text-xs font-mono" />
                </Field>
              </div>
            </Group>
            <Advanced label="思考等级映射">
              <div className="grid grid-cols-2 gap-3">
                {THINKING_LEVELS.map((l) => (
                  <Field key={l.value} label={l.label}>
                    <Input value={mThink[l.value] ?? ""} onChange={(e) => setMThink({ ...mThink, [l.value]: e.target.value })}
                      placeholder={l.value} className="h-8 text-xs font-mono" />
                  </Field>
                ))}
              </div>
              <p className="text-[11px] text-neutral-500">
                思考等级映射：留空 = 不映射（按提供商默认传参）
              </p>
            </Advanced>
          </div>
      </FormDialog>
    </div>
  );
}
