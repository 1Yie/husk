import { memo, useEffect, useMemo, useRef, useState } from "react";
import { cjk } from "@streamdown/cjk";
import { createMathPlugin } from "@streamdown/math";
import {
  Streamdown,
  type CustomRenderer,
  type CustomRendererProps,
  type DiagramPlugin,
  type StreamdownTranslations,
} from "streamdown";
import { Check, Copy } from "@keyline-icons/react";
import { chatMarkdownComponents } from "./markdown-components";
import { codeHighlightPlugin, codeHighlightPluginStreaming } from "../../lib/syntax-highlight";
import { DiffView } from "../diff-view";

/** Streamdown's toolbar icons — module-level so the renderer's `icons` prop
 *  stays referentially stable (a fresh object defeats Streamdown's own
 *  per-block memo). */
export const streamdownIcons = {
  CheckIcon: Check,
  CopyIcon: Copy,
};

/** streamdown's own labels are English (copy/download, table export, diagram controls) — untranslated they leak into a Chinese UI. */
export const streamdownTranslations: Partial<StreamdownTranslations> = {
  close: "关闭",
  copied: "已复制",
  copyCode: "复制代码",
  copyLink: "复制链接",
  copyTable: "复制表格",
  copyTableAsCsv: "复制为 CSV",
  copyTableAsMarkdown: "复制为 Markdown",
  copyTableAsTsv: "复制为 TSV",
  downloadDiagram: "下载图表",
  downloadDiagramAsMmd: "Mermaid 源码",
  downloadDiagramAsPng: "PNG 图片",
  downloadDiagramAsSvg: "SVG 图片",
  downloadFile: "下载文件",
  downloadImage: "下载图片",
  downloadTable: "下载表格",
  downloadTableAsCsv: "下载 CSV",
  downloadTableAsMarkdown: "下载 Markdown",
  exitFullscreen: "退出全屏",
  externalLinkWarning: "即将离开本应用访问外部网站。",
  imageNotAvailable: "图片无法显示",
  mermaidFormatMmd: "MMD",
  mermaidFormatPng: "PNG",
  mermaidFormatSvg: "SVG",
  openExternalLink: "打开外部链接？",
  openLink: "打开链接",
  resetView: "重置缩放",
  tableFormatCsv: "CSV",
  tableFormatMarkdown: "Markdown",
  tableFormatTsv: "TSV",
  viewFullscreen: "全屏查看",
  zoomIn: "放大",
  zoomOut: "缩小",
};

/** Code blocks get no download button — expressed here so it never mounts. */
const streamdownControls = { code: { download: false } } as const;

/**
 * ```diff/```patch render through the approval cards' `DiffView`, plus the copy
 * affordance `DiffView` lacks. A half-arrived fence stays a plain preview: it
 * would parse to nothing and `DiffView` renders `null` for that.
 */
function DiffFence({ code, isIncomplete }: CustomRendererProps) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timer.current), []);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => setCopied(false), 1600);
    } catch {
      // Clipboard denied — the patch stays selectable, which is the fallback.
    }
  };

  if (isIncomplete) {
    return (
      <pre className="w-full rounded-xl border border-neutral-200 bg-neutral-50 px-3 py-2.5 font-mono text-[12px] leading-relaxed text-neutral-700 whitespace-pre-wrap break-all select-text">
        {code}
      </pre>
    );
  }
  return (
    <div className="group/diff relative">
      <DiffView diff={code} maxHeight={420} />
      <button
        type="button"
        aria-label={copied ? "已复制" : "复制补丁"}
        onClick={() => void copy()}
        className="absolute right-2 top-2 z-10 flex items-center gap-1 rounded-md border border-hairline bg-panel/90 px-1.5 py-1 text-[11px] text-neutral-500 opacity-0 transition-opacity group-hover/diff:opacity-100 hover:text-neutral-800 cursor-pointer supports-[backdrop-filter]:bg-panel/70"
      >
        {copied ? <Check className="h-3 w-3" /> : <Copy className="h-3 w-3" />}
        <span>{copied ? "已复制" : "复制"}</span>
      </button>
    </div>
  );
}

/** Fence language → component, for fences that are not code. */
export const streamdownRenderers: CustomRenderer[] = [
  { language: ["diff", "patch"], component: DiffFence },
];

/** `$$…$$` always renders; `$…$` is on because these models write inline math far more often than prices. */
const streamdownMath = createMathPlugin({ singleDollarTextMath: true });

/** Lazy: ~1 MB of diagram engine for fences most answers never contain (fetched on idle, first diagram-looking fence). */
const MERMAID_FENCE = /^[ \t]*(?:```|~~~)[ \t]*mermaid\b/m;
let mermaidPluginPromise: Promise<DiagramPlugin> | null = null;
function loadMermaidPlugin(): Promise<DiagramPlugin> {
  // Theme baked into the plugin: passing it as the `mermaid` prop re-runs mermaid's global `initialize` on every render.
  mermaidPluginPromise ??= import("@streamdown/mermaid").then((m) =>
    m.createMermaidPlugin({ config: { theme: "neutral" } }),
  );
  return mermaidPluginPromise;
}

/**
 * Streaming fade (inert unless `isAnimating`): `blurIn`, 200 ms, word-level with
 * `maxBacklogMs: 120` — char-level measured 16.4 ms per commit against a 16.7 ms
 * frame budget. `streamFx` picks the surface: the answer opts in, a reasoning
 * trace stays static (18 ms/commit over 12k nodes when animated). No caret.
 */
const streamdownAnimate = {
  animation: "blurIn",
  duration: 200,
  easing: "ease-out",
  sep: "word",
  maxBacklogMs: 120,
} as const;

export type StreamFx = "animate" | "none";

/** Streamdown in `memo`: a finished block's `text` reference is stable, so only
 * the block whose text changed re-renders. `plugins` is memoized on `animating` — a
 * fresh object would defeat Streamdown's own per-block memo (its comparator includes
 * `plugins`) and re-render every code block; while streaming it also selects the
 * complete-lines-only highlighter.
 *
 * Shared by the answer text, the reasoning panel and tool prose — the caller picks
 * the block styling through `components`. */
export const MemoStreamdown = memo(function MemoStreamdown({
  text,
  animating,
  components = chatMarkdownComponents,
  streamFx = "none",
}: {
  text: string;
  animating?: boolean;
  /** Block styling map. Defaults to the answer's; the reasoning panel passes
   *  a muted variant so expanded thinking doesn't read as answer text. */
  components?: typeof chatMarkdownComponents;
  /** Streaming fade for this surface. Off by default; the answer opts in. */
  streamFx?: StreamFx;
}) {
  const needsMermaid = useMemo(() => MERMAID_FENCE.test(text), [text]);
  const [mermaidPlugin, setMermaidPlugin] = useState<DiagramPlugin | null>(null);
  useEffect(() => {
    if (!needsMermaid || mermaidPlugin) return;
    let alive = true;
    void loadMermaidPlugin().then((plugin) => {
      if (alive) setMermaidPlugin(plugin);
    });
    return () => {
      alive = false;
    };
  }, [needsMermaid, mermaidPlugin]);

  const plugins = useMemo(
    () => ({
      cjk,
      math: streamdownMath,
      code: (animating ? codeHighlightPluginStreaming : codeHighlightPlugin) as any,
      renderers: streamdownRenderers,
      ...(mermaidPlugin ? { mermaid: mermaidPlugin } : {}),
    }),
    [animating, mermaidPlugin],
  );
  return (
    <Streamdown
      isAnimating={animating}
      plugins={plugins}
      shikiTheme={["github-dark", "github-dark"]}
      components={components}
      icons={streamdownIcons}
      translations={streamdownTranslations}
      controls={streamdownControls}
      animated={streamFx === "animate" ? streamdownAnimate : undefined}
    >
      {text}
    </Streamdown>
  );
});
