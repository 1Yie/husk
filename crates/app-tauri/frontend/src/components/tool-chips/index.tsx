import { useState } from "react";
import { DiffView } from "../diff-view";
import { TodoView } from "../todo-view";

function asChipText(value: unknown): string {
  if (typeof value === "string") return value;
  if (value instanceof Date && !Number.isNaN(value.getTime())) {
    return `${value.getFullYear()}年${value.getMonth() + 1}月${value.getDate()}日`;
  }
  return String(value ?? "");
}

function formatChipArgs(label: string, chip: string): string {
  if (!chip) return "";
  if (label === "apply_patch") {
    // Extract file paths from patch header lines
    const files: string[] = [];
    const re = /(?:\*\*\*\s*(?:Add|Update|Delete)\s*File:\s*|^(?:added|updated|deleted)\s+)([^\s\n\r]+)/gim;
    let match: RegExpExecArray | null;
    while ((match = re.exec(chip)) !== null) {
      if (match[1] && !files.includes(match[1])) {
        files.push(match[1]);
      }
    }
    if (files.length > 0) {
      return files.join(", ");
    }
  }
  if (label === "todo") {
    try {
      const parsed = JSON.parse(chip);
      if (parsed.action) {
        if (parsed.action === "add" && parsed.text) return `add: ${parsed.text}`;
        if (parsed.id) return `${parsed.action}: #${parsed.id}`;
        return parsed.action;
      }
    } catch {
      // Plain string
      return chip;
    }
  }
  return chip;
}

function isDiffText(text: string): boolean {
  if (!text) return false;
  return (
    (text.includes("--- ") && text.includes("+++ ")) ||
    text.includes("@@ -") ||
    text.startsWith("*** Begin Patch") ||
    text.includes("diff --git ") ||
    (text.includes("added ") && text.includes("@@")) ||
    (text.includes("updated ") && text.includes("@@")) ||
    (text.includes("deleted ") && text.includes("@@"))
  );
}

export type ToolChipRow = {
  chip: string;
  detail?: string[];
  id: string;
  label: string;
  status: "aborted" | "done" | "running";
  uiType?: string;
  approval?: {
    requestId: number;
    diff: string;
    resolved?: boolean;
    approved?: boolean;
  };
};

export function ToolChips({ rows }: { rows: ToolChipRow[] }) {
  const [open, setOpen] = useState(true);
  const [openRows, setOpenRows] = useState<Set<string>>(new Set());
  const [closedRows, setClosedRows] = useState<Set<string>>(new Set());
  const running = rows.filter((row) => row.status === "running").length;

  const isRowOpen = (row: ToolChipRow) => {
    if (closedRows.has(row.id)) return false;
    if (openRows.has(row.id)) return true;
    // Auto-open if awaiting approval or active todo list
    if (row.approval && !row.approval.resolved) return true;
    if (row.label === "todo" || row.uiType === "todo") return true;
    return false;
  };

  const toggleRow = (id: string, currentlyOpen: boolean) => {
    if (currentlyOpen) {
      setOpenRows((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
      setClosedRows((prev) => new Set(prev).add(id));
    } else {
      setClosedRows((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
      setOpenRows((prev) => new Set(prev).add(id));
    }
  };

  if (rows.length === 0) return null;

  return (
    <div className="w-full my-1">
      <button
        aria-expanded={open}
        className="text-neutral-500 hover:bg-neutral-100 dark:hover:bg-neutral-800 flex w-fit items-center gap-1.5 rounded-md px-1.5 py-1 text-xs cursor-pointer select-none transition-colors"
        onClick={() => {
          setOpen((current) => !current);
        }}
        type="button"
      >
        <svg
          aria-hidden="true"
          className="size-3 transition-transform duration-200"
          fill="none"
          stroke="currentColor"
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth="2.2"
          style={{ transform: open ? "rotate(0deg)" : "rotate(-90deg)" }}
          viewBox="0 0 24 24"
        >
          <path d="M6 9l6 6 6-6" />
        </svg>
        <span>
          {rows.length} 次工具调用
          {running > 0 ? ` · ${running} 进行中` : ""}
        </span>
      </button>
      <div
        className="grid transition-[grid-template-rows,opacity] duration-300 w-full"
        style={{
          gridTemplateRows: open ? "1fr" : "0fr",
          opacity: open ? 1 : 0,
        }}
      >
        <div className="overflow-hidden w-full">
          <div className="mt-1.5 flex flex-col gap-1.5 w-full">
            {rows.map((row) => {
              const rowOpen = isRowOpen(row);
              const statusText =
                row.approval && !row.approval.resolved
                  ? "待批准"
                  : row.status === "aborted" || (row.approval?.resolved && !row.approval?.approved)
                    ? "已中断"
                    : row.status === "running"
                      ? "进行中"
                      : "完成";

              const detailText = row.detail ? row.detail.join("\n").trim() : "";
              const isDiff =
                row.uiType === "diff" ||
                Boolean(row.approval?.diff) ||
                isDiffText(detailText);

              const diffContent =
                isDiff && detailText.length > 0
                  ? detailText
                  : (row.approval?.diff ?? (isDiff ? detailText : null));

              const isTodo =
                row.label === "todo" ||
                row.uiType === "todo" ||
                (detailText.includes("Todo List:") && detailText.includes("[")) ||
                /\[[ xX]\]\s*#?\d+/i.test(detailText);

              const hasDetail = Boolean(diffContent) || detailText.length > 0;
              const chipText = formatChipArgs(row.label, row.chip);

              return (
                <div key={row.id} className="w-full">
                  <button
                    aria-expanded={rowOpen}
                    className="hover:bg-neutral-100 dark:hover:bg-neutral-800 flex h-7 w-fit max-w-full items-center gap-2 rounded-md px-1 text-left select-none cursor-pointer transition-colors"
                    onClick={() => {
                      if (hasDetail) toggleRow(row.id, rowOpen);
                    }}
                    type="button"
                  >
                    {/* 工具名称 */}
                    <span className="text-neutral-500 shrink-0 text-xs font-medium">
                      {asChipText(row.label)}
                    </span>
                    {/* args */}
                    {chipText ? (
                      <span className="bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-300 inline-flex h-5 items-center rounded-md px-1.5 font-mono text-[11px] truncate max-w-[400px]">
                        {asChipText(chipText)}
                      </span>
                    ) : null}
                    {/* 状态 */}
                    <span className="text-neutral-400 shrink-0 text-[11px]">
                      {statusText}
                    </span>
                  </button>

                  {hasDetail ? (
                    <div
                      className="grid transition-[grid-template-rows,opacity] duration-300 w-full"
                      style={{
                        gridTemplateRows: rowOpen ? "1fr" : "0fr",
                        opacity: rowOpen ? 1 : 0,
                      }}
                    >
                      <div className="min-h-0 overflow-hidden w-full">
                        <div className="w-full my-1.5">
                          {diffContent ? (
                            <DiffView diff={diffContent} maxHeight={600} />
                          ) : isTodo ? (
                            <TodoView content={detailText} />
                          ) : (
                            <div className="w-full rounded-xl border border-neutral-200 dark:border-neutral-800 bg-[#fafafa] dark:bg-[#121214] p-3 font-mono text-[11.5px] text-neutral-700 dark:text-neutral-300 leading-relaxed overflow-x-auto max-h-[400px] overflow-y-auto select-text shadow-2xs">
                              {row.detail?.map((line, idx) => (
                                <div key={idx} className="whitespace-pre-wrap break-all">
                                  {asChipText(line)}
                                </div>
                              ))}
                            </div>
                          )}
                        </div>
                      </div>
                    </div>
                  ) : null}
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}

export const ToolTimeline = ToolChips;
