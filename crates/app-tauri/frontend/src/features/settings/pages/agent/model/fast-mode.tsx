import { useId } from "react";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Field, SettingSelect } from "@/features/settings/components";
import { AdvancedFields } from "@/features/settings/components/advanced-fields";
import type { FastModeConfig } from "@/lib/agent-ipc";

/** Keep incomplete JSON and model IDs local until the model form is saved. */
export interface FastModeDraft {
  enabled: boolean;
  mode: FastModeConfig["mode"];
  model: string;
  params: string;
  headers: string;
}

export function fastModeDraft(config?: FastModeConfig, providerKind?: string): FastModeDraft {
  return {
    enabled: config != null,
    mode: config?.mode ?? (providerKind === "anthropic" ? "speed"
      : providerKind === "openai_responses" || providerKind === "openai_completions" || providerKind === "openai_codex_responses"
        ? "priority" : "custom"),
    model: config?.model ?? "",
    params: config?.params == null ? "" : JSON.stringify(config.params, null, 2),
    headers: config?.headers == null ? "" : JSON.stringify(config.headers, null, 2),
  };
}

function parseObject(text: string, label: string): Record<string, unknown> | undefined {
  if (!text.trim()) return undefined;
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    throw new Error(`${label}必须是有效的 JSON 对象`);
  }
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label}必须是 JSON 对象，不能是数组、null 或其他值`);
  }
  return value as Record<string, unknown>;
}

export function validateFastMode(draft: FastModeDraft):
  { config?: FastModeConfig; error?: undefined } | { error: string; config?: undefined } {
  if (!draft.enabled) return {};
  if (draft.mode === "model" && !draft.model.trim()) {
    return { error: "请填写同一提供商的快速版模型 ID" };
  }
  try {
    const params = parseObject(draft.params, "Fast 参数");
    const headers = parseObject(draft.headers, "Fast 请求头");
    if (headers && Object.values(headers).some((value) => typeof value !== "string")) {
      return { error: "Fast 请求头的值必须全部为字符串" };
    }
    if (params && ("fastMode" in params || "fast_mode" in params)) {
      return { error: "Fast 参数不能包含应用内部配置 fastMode / fast_mode" };
    }
    if (params && "model" in params) {
      return { error: "请使用「快速版模型」配置模型 ID，不要在 Fast 参数中设置 model" };
    }
    if (headers && Object.entries(headers).some(([name, value]) =>
      !/^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/.test(name)
      || Array.from(value as string).some((char) =>
        (char.charCodeAt(0) < 32 && char !== "\t") || char.charCodeAt(0) === 127))) {
      return { error: "Fast 请求头名称必须有效，值不能包含换行或非法控制字符" };
    }
    if (draft.mode === "custom" && !Object.keys(params ?? {}).length && !Object.keys(headers ?? {}).length) {
      return { error: "请填写非空的自定义 Fast 参数或请求头 JSON 对象" };
    }
    return {
      config: {
        mode: draft.mode,
        ...(draft.mode === "model" ? { model: draft.model.trim() } : {}),
        ...(params ? { params } : {}),
        ...(headers ? { headers: headers as Record<string, string> } : {}),
      },
    };
  } catch (error) {
    return { error: (error as Error).message };
  }
}

const MODE_OPTIONS = [
  { value: "off", label: "不开启" },
  { value: "priority", label: "service_tier: priority" },
  { value: "speed", label: "speed: fast" },
  { value: "model", label: "model" },
  { value: "custom", label: "自定义参数" },
];

export function FastModeFields({ value, onChange }: {
  value: FastModeDraft;
  onChange: (draft: FastModeDraft) => void;
}) {
  const errorId = useId();
  const { error } = validateFastMode(value);
  const update = (patch: Partial<FastModeDraft>) => onChange({ ...value, ...patch });
  const custom = value.mode === "custom";
  const protocolFields = (
    <div className="flex min-w-0 flex-col gap-3">
      <Field label={custom ? "请求参数" : "附加参数（可选）"}>
        <Textarea
          aria-label="Fast 参数 JSON"
          aria-describedby={error ? errorId : undefined}
          value={value.params}
          onChange={(event) => update({ params: event.target.value })}
          placeholder="{}"
          rows={3}
          className="max-h-40 text-xs font-mono"
        />
      </Field>
      <Field label={custom ? "请求头" : "附加请求头（可选）"}>
        <Textarea
          aria-label="Fast 请求头 JSON"
          aria-describedby={error ? errorId : undefined}
          value={value.headers}
          onChange={(event) => update({ headers: event.target.value })}
          placeholder="{}"
          rows={3}
          className="max-h-40 text-xs font-mono"
        />
      </Field>
    </div>
  );

  return (
    <section aria-label="Fast 模式配置" className="flex min-w-0 flex-col gap-2.5">
      <div className="grid grid-cols-2 items-center gap-3">
        <span className="text-xs font-medium text-neutral-700">Fast 模式</span>
        <SettingSelect
          ariaLabel="Fast 模式"
          value={value.enabled ? value.mode : "off"}
          onChange={(mode) => update(mode === "off"
            ? { enabled: false }
            : { enabled: true, mode: mode as FastModeConfig["mode"] })}
          options={MODE_OPTIONS}
          triggerClassName="w-full min-w-0"
        />
      </div>
      {value.enabled ? (
        <div className="flex min-w-0 flex-col gap-3">
          {value.mode === "model" && (
            <Field label="快速版模型 ID">
              <Input
                aria-label="快速版模型 ID"
                aria-invalid={!value.model.trim()}
                aria-describedby={error ? errorId : undefined}
                value={value.model}
                onChange={(event) => update({ model: event.target.value })}
                placeholder="提供商的完整模型 ID"
                className="h-8 text-xs font-mono"
              />
            </Field>
          )}
          {custom ? protocolFields : (
            <AdvancedFields label="高级参数" defaultOpen={Boolean(value.params.trim() || value.headers.trim())}>
              {protocolFields}
            </AdvancedFields>
          )}
          {error && <p id={errorId} role="alert" className="text-xs text-red-600">{error}</p>}
        </div>
      ) : null}
    </section>
  );
}
