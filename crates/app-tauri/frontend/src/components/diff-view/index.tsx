// DiffView — Full-width modern unified diff renderer with dark-mode support,
// multi-file patch header awareness, hunk styling, and scrollable container.

import { useMemo } from "react";

interface Props {
  diff: string;
  maxHeight?: number;
}

type LineKind = "add" | "del" | "context" | "hunk" | "file_header" | "meta";

interface DLine {
  kind: LineKind;
  text: string;
  path?: string;
}

export function DiffView({ diff, maxHeight = 500 }: Props) {
  const lines = useMemo(() => parseDiff(diff), [diff]);

  if (!diff || lines.length === 0) return null;

  return (
    <div className="w-full rounded-xl border border-neutral-200 dark:border-neutral-800 bg-[#fafafa] dark:bg-[#121214] overflow-hidden shadow-2xs font-mono text-[12px] leading-relaxed select-text">
      <div
        className="w-full overflow-x-auto overflow-y-auto"
        style={{ maxHeight }}
      >
        <div className="min-w-full w-max py-1">
          {lines.map((l, i) => {
            if (l.kind === "file_header") {
              return (
                <div
                  key={i}
                  className="px-3 pt-2.5 pb-1 font-mono text-[12px] font-semibold text-neutral-700 dark:text-neutral-300 select-text"
                >
                  <span className="truncate font-mono">
                    {l.path || l.text}
                  </span>
                </div>
              );
            }

            if (l.kind === "hunk") {
              return (
                <div
                  key={i}
                  className="px-3 py-0.5 my-0.5 bg-blue-500/10 text-blue-600 dark:text-blue-400 text-[11px] font-medium select-none"
                >
                  <span>{l.text}</span>
                </div>
              );
            }

            return (
              <div
                key={i}
                className={`flex items-start px-2 py-[1px] transition-colors ${bgClass(l.kind)} ${fgClass(l.kind)}`}
              >
                <span
                  className={`flex-none w-5 text-center select-none font-bold text-[11px] opacity-75 ${
                    l.kind === "add"
                      ? "text-emerald-600 dark:text-emerald-400"
                      : l.kind === "del"
                      ? "text-rose-600 dark:text-rose-400"
                      : "text-neutral-400"
                  }`}
                >
                  {gutter(l.kind)}
                </span>
                <span className="flex-1 min-w-0 font-mono whitespace-pre">
                  {l.text || "\u00A0"}
                </span>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function parseFilePath(raw: string): string {
  const trimmed = raw.trim();
  if (trimmed.startsWith("added ") || trimmed.startsWith("*** Add File:")) {
    return trimmed.replace(/^added\s+/, "").replace(/^\*\*\*\s*Add File:\s*/, "");
  }
  if (trimmed.startsWith("deleted ") || trimmed.startsWith("*** Delete File:")) {
    return trimmed.replace(/^deleted\s+/, "").replace(/^\*\*\*\s*Delete File:\s*/, "");
  }
  if (
    trimmed.startsWith("updated ") ||
    trimmed.startsWith("modified ") ||
    trimmed.startsWith("patched ") ||
    trimmed.startsWith("*** Update File:")
  ) {
    return trimmed
      .replace(/^updated\s+/, "")
      .replace(/^modified\s+/, "")
      .replace(/^patched\s+/, "")
      .replace(/^\*\*\*\s*Update File:\s*/, "");
  }
  if (trimmed.startsWith("diff --git")) {
    const parts = trimmed.split(/\s+/);
    const bPart = parts.find((p) => p.startsWith("b/"));
    return bPart ? bPart.slice(2) : trimmed;
  }
  return trimmed;
}

function parseDiff(diff: string): DLine[] {
  const out: DLine[] = [];
  const rawLines = diff.split("\n");

  for (let i = 0; i < rawLines.length; i++) {
    const raw = rawLines[i];
    const trimmed = raw.trim();

    // Skip envelope markers
    if (trimmed === "*** Begin Patch" || trimmed === "*** End Patch") {
      continue;
    }

    // Skip dummy diff headers if they refer to generic a/file or b/file
    if (trimmed === "--- a/file" || trimmed === "+++ b/file") {
      continue;
    }

    // Check for multi-file header indicators
    if (
      trimmed.startsWith("diff --git") ||
      trimmed.startsWith("Index:") ||
      trimmed.startsWith("index ") ||
      trimmed.startsWith("added ") ||
      trimmed.startsWith("deleted ") ||
      trimmed.startsWith("modified ") ||
      trimmed.startsWith("patched ") ||
      trimmed.startsWith("updated ") ||
      trimmed.startsWith("*** Add File:") ||
      trimmed.startsWith("*** Update File:") ||
      trimmed.startsWith("*** Delete File:")
    ) {
      const path = parseFilePath(raw);
      out.push({ kind: "file_header", text: raw, path });
      continue;
    }

    if (raw.startsWith("@@")) {
      out.push({ kind: "hunk", text: raw });
    } else if (raw.startsWith("+++") || raw.startsWith("---")) {
      out.push({ kind: "meta", text: raw });
    } else if (raw.startsWith("+")) {
      out.push({ kind: "add", text: raw.slice(1) });
    } else if (raw.startsWith("-")) {
      out.push({ kind: "del", text: raw.slice(1) });
    } else if (raw.startsWith("\\ No newline")) {
      out.push({ kind: "meta", text: raw });
    } else {
      // If an empty line immediately follows a file header or another empty line at start, omit it
      if (trimmed === "") {
        const lastKind = out[out.length - 1]?.kind;
        if (!lastKind || lastKind === "file_header") {
          continue;
        }
      }
      out.push({ kind: "context", text: raw });
    }
  }

  return out;
}

function gutter(k: LineKind) {
  return k === "add" ? "+" : k === "del" ? "-" : " ";
}

function bgClass(k: LineKind) {
  switch (k) {
    case "add":
      return "bg-emerald-500/10 dark:bg-emerald-500/15";
    case "del":
      return "bg-rose-500/10 dark:bg-rose-500/15";
    case "meta":
      return "bg-neutral-100/50 dark:bg-neutral-800/40";
    default:
      return "";
  }
}

function fgClass(k: LineKind) {
  switch (k) {
    case "add":
      return "text-emerald-800 dark:text-emerald-300";
    case "del":
      return "text-rose-800 dark:text-rose-300";
    case "meta":
      return "text-neutral-400 dark:text-neutral-500 italic";
    default:
      return "text-neutral-800 dark:text-neutral-200";
  }
}
