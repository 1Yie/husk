import { useId } from "react";
import { ArrowDown, ArrowUp, Plus, Route, X } from "@keyline-icons/react";
import { Button } from "@/components/ui/button";
import { KvList } from "@/components/ui/kv-list";
import { SkeletonKvRows } from "@/components/ui/skeleton";
import { TooltipSimple } from "@/components/ui/tooltip";
import { ItemIcon, SettingSelect, Tag } from "@/features/settings/components";

interface FallbackProvider {
  kind?: string;
  type?: string;
  api?: string;
  default_model?: string | null;
  defaultModel?: string | null;
}

export function FallbackChain({
  chain,
  providers,
  loading,
  busy,
  disabled,
  onChange,
  onAddProvider,
}: {
  chain: string[];
  providers: Record<string, FallbackProvider>;
  loading: boolean;
  busy: boolean;
  disabled: boolean;
  onChange: (next: string[]) => void;
  onAddProvider: () => void;
}) {
  const titleId = useId();
  const candidates = Object.keys(providers).filter((name) => !chain.includes(name));
  const locked = loading || busy || disabled;

  const move = (index: number, to: number) => {
    if (locked || to < 0 || to >= chain.length) return;
    const next = [...chain];
    [next[index], next[to]] = [next[to], next[index]];
    onChange(next);
  };

  return (
    <section aria-labelledby={titleId}>
      <KvList>
        <div className="flex items-start gap-3 px-5 py-4">
          <ItemIcon><Route className="h-4 w-4" /></ItemIcon>
          <div className="flex min-w-0 flex-1 flex-col gap-1">
            <div className="flex flex-wrap items-center gap-2">
              <h2 id={titleId} className="text-sm font-semibold text-neutral-900">模型降级链</h2>
              {!loading && <Tag className="tabular-nums">{chain.length ? `${chain.length} 个备用` : "未设置"}</Tag>}
            </div>
            <p className="text-xs leading-relaxed text-neutral-500">主提供商重试失败后，按此顺序切换</p>
          </div>
          {busy && <span role="status" className="shrink-0 text-[11px] text-neutral-500">保存中…</span>}
        </div>

        {loading ? <SkeletonKvRows rows={2} /> : chain.length ? (
          <ol className="max-h-64 overflow-y-auto border-t border-hairline divide-y divide-neutral-100 dark:divide-[var(--husk-border)]">
            {chain.map((name, index) => {
              const provider = providers[name];
              const model = provider?.default_model ?? provider?.defaultModel;
              return (
                <li key={`${name}:${index}`} className="flex items-center gap-3 px-5 py-3">
                  <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-neutral-100 text-xs font-medium tabular-nums text-neutral-600">
                    {index + 1}
                  </span>
                  <div className="flex min-w-0 flex-1 flex-col gap-1">
                    <span title={name} className="truncate text-[13px] font-medium text-neutral-800">{name}</span>
                    {provider ? (
                      <span title={model || undefined} className="truncate text-[11px] text-neutral-500">
                        {model || provider.kind || provider.type || provider.api || "openai_completions"}
                      </span>
                    ) : <span className="text-[11px] text-amber-600 dark:text-amber-400">提供商已不存在</span>}
                  </div>
                  <div className="flex shrink-0 items-center gap-0.5 rounded-full bg-neutral-100 p-0.5">
                    <TooltipSimple content="上移">
                      <Button size="sm" variant="ghost" aria-label={`上移 ${name}`}
                        className="h-7 w-7 rounded-full p-0 text-neutral-500 hover:bg-white"
                        disabled={locked || index === 0} onClick={() => move(index, index - 1)}>
                        <ArrowUp className="h-3.5 w-3.5" />
                      </Button>
                    </TooltipSimple>
                    <TooltipSimple content="下移">
                      <Button size="sm" variant="ghost" aria-label={`下移 ${name}`}
                        className="h-7 w-7 rounded-full p-0 text-neutral-500 hover:bg-white"
                        disabled={locked || index === chain.length - 1} onClick={() => move(index, index + 1)}>
                        <ArrowDown className="h-3.5 w-3.5" />
                      </Button>
                    </TooltipSimple>
                    <TooltipSimple content="移出降级链">
                      <Button size="sm" variant="ghost" aria-label={`移除 ${name}`}
                        className="h-7 w-7 rounded-full p-0 text-neutral-500 hover:bg-white hover:text-red-600"
                        disabled={locked} onClick={() => onChange(chain.filter((_, i) => i !== index))}>
                        <X className="h-3.5 w-3.5" />
                      </Button>
                    </TooltipSimple>
                  </div>
                </li>
              );
            })}
          </ol>
        ) : (
          <div className="px-5 pb-4 text-xs text-neutral-500">
            {Object.keys(providers).length ? "添加备用提供商，设置自动切换顺序" : "添加提供商后即可配置备用顺序"}
          </div>
        )}

        <div className="border-t border-hairline px-5 py-3">
          {candidates.length ? (
            <SettingSelect
              value=""
              placeholder="添加备用提供商"
              ariaLabel="添加备用提供商"
              prefix={<Plus className="h-3.5 w-3.5 shrink-0" />}
              disabled={locked}
              triggerClassName="w-full min-w-0 shadow-none"
              contentClassName="max-h-60 overflow-y-auto"
              options={candidates.map((name) => ({ value: name, label: name }))}
              onChange={(name) => {
                if (!locked && candidates.includes(name)) onChange([...chain, name]);
              }}
            />
          ) : Object.keys(providers).length ? (
            <span className="text-xs text-neutral-500">所有提供商均已加入</span>
          ) : (
            <Button size="sm" variant="outline" className="h-7 gap-1 text-xs" disabled={locked} onClick={onAddProvider}>
              <Plus className="h-3.5 w-3.5" /> 添加 API
            </Button>
          )}
        </div>
      </KvList>
    </section>
  );
}
