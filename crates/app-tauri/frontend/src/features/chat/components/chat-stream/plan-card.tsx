// The plan-mode deliverable card — rendered from the `submit_plan` contract
// payload (live `PlanSubmitted` event or the persisted `NoticeKind::Plan`
// row on replay). Always expanded: the plan IS the answer, not a footnote.
import { memo } from "react";
import { ClipboardList, CheckCircle2, AlertTriangle, FileCode2 } from "lucide-react";
import type { PlanPayload } from "@/types";
import { MemoStreamdown } from "@/features/chat/components/chat-stream/markdown-stream";

export const PlanCard = memo(function PlanCard({ plan }: { plan: PlanPayload }) {
  const steps = plan.steps ?? [];
  const verification = plan.verification ?? [];
  const risks = plan.risks ?? [];
  return (
    <div className="my-1 w-full min-w-0 overflow-hidden rounded-lg border border-[color-mix(in_srgb,var(--husk-n300)_75%,transparent)] bg-[color-mix(in_srgb,var(--husk-n100)_60%,transparent)]">
      {/* Header — the plan's one-paragraph outcome. */}
      <div className="flex items-start gap-2 px-3 pt-2.5 pb-1.5">
        <ClipboardList className="mt-0.5 h-3.5 w-3.5 shrink-0 text-neutral-500" />
        <div className="min-w-0 flex-1">
          <span className="text-[12px] font-semibold text-neutral-800">
            实施方案
          </span>
          <div className="select-text text-[13px] leading-relaxed text-neutral-700">
            <MemoStreamdown text={plan.summary} animating={false} />
          </div>
        </div>
      </div>

      {/* Ordered steps — each carries the files it touches. */}
      {steps.length > 0 && (
        <ol className="flex flex-col gap-1.5 px-3 pb-2">
          {steps.map((s, i) => (
            <li key={i} className="flex items-start gap-2">
              <span className="mt-px flex h-4 w-4 shrink-0 items-center justify-center rounded-full bg-[color-mix(in_srgb,var(--husk-n300)_60%,transparent)] font-mono text-[10px] text-neutral-600 tabular-nums">
                {i + 1}
              </span>
              <div className="min-w-0 flex-1">
                <div className="select-text text-[13px] font-medium text-neutral-800 leading-snug">
                  {s.title}
                </div>
                {s.detail && (
                  <div className="select-text text-[12px] leading-relaxed text-neutral-600">
                    <MemoStreamdown text={s.detail} animating={false} />
                  </div>
                )}
                {s.files && s.files.length > 0 && (
                  <div className="mt-0.5 flex flex-wrap gap-1">
                    {s.files.map((f) => (
                      <span
                        key={f}
                        className="inline-flex items-center gap-1 rounded bg-[color-mix(in_srgb,var(--husk-n200)_55%,transparent)] px-1.5 py-px font-mono text-[10px] text-neutral-600"
                      >
                        <FileCode2 className="h-2.5 w-2.5" />
                        {f}
                      </span>
                    ))}
                  </div>
                )}
              </div>
            </li>
          ))}
        </ol>
      )}

      {/* Verification + risks — two slim footer sections. */}
      {verification.length > 0 && (
        <div className="border-t border-[color-mix(in_srgb,var(--husk-n300)_50%,transparent)] px-3 py-1.5">
          <div className="flex items-center gap-1.5 text-[11px] font-medium text-neutral-600">
            <CheckCircle2 className="h-3 w-3" />
            验证
          </div>
          <ul className="mt-0.5 flex flex-col gap-0.5">
            {verification.map((v, i) => (
              <li
                key={i}
                className="select-text pl-4 font-mono text-[11px] text-neutral-600"
              >
                {v}
              </li>
            ))}
          </ul>
        </div>
      )}
      {risks.length > 0 && (
        <div className="border-t border-[color-mix(in_srgb,var(--husk-n300)_50%,transparent)] px-3 py-1.5">
          <div className="flex items-center gap-1.5 text-[11px] font-medium text-amber-600">
            <AlertTriangle className="h-3 w-3" />
            风险与待定
          </div>
          <ul className="mt-0.5 flex flex-col gap-0.5">
            {risks.map((r, i) => (
              <li key={i} className="select-text pl-4 text-[11px] text-neutral-600">
                {r}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
});

/** Serialize the structured plan back to markdown — the approve prompt and
 * the copy button both hand the executor this exact text. */
export function planToMarkdown(plan: PlanPayload): string {
  const lines: string[] = [`## 实施方案\n`, plan.summary, `\n### 步骤`];
  (plan.steps ?? []).forEach((s, i) => {
    lines.push(`${i + 1}. **${s.title}**`);
    if (s.detail) lines.push(`   ${s.detail}`);
    if (s.files?.length) lines.push(`   文件：${s.files.join("、")}`);
  });
  if (plan.verification?.length) {
    lines.push(`\n### 验证`);
    plan.verification.forEach((v) => lines.push(`- ${v}`));
  }
  if (plan.risks?.length) {
    lines.push(`\n### 风险与待定`);
    plan.risks.forEach((r) => lines.push(`- ${r}`));
  }
  return lines.join("\n");
}
