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

/** Streamdown ships English labels for everything it draws itself — copy and
 *  download buttons, the table export menu, diagram controls, the external-link
 *  confirmation. Untranslated they leak English into an otherwise Chinese UI. */
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

/** The download button is not wanted on chat code blocks (the patch/output is
 *  the conversation, not a file to save) — expressed here instead of hiding the
 *  rendered button from CSS, so it never mounts in the first place. */
const streamdownControls = { code: { download: false } } as const;

/** `diff` fences are the one fence the app renders better than a coloured code
 *  block: the same `DiffView` the approval cards use, plus the copy affordance
 *  `DiffView` itself does not carry (a patch in an answer is usually there to be
 *  copied). While the fence is still arriving it stays a plain preview — a patch
 *  cut mid-hunk would parse to nothing and `DiffView` renders `null` for that,
 *  leaving a hole in the answer's layout. */
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

/** Fence-language → React component, for fences that are not code. `patch` is
 *  the same payload under the other common spelling. */
export const streamdownRenderers: CustomRenderer[] = [
  { language: ["diff", "patch"], component: DiffFence },
];

/** LaTeX: `$$…$$` blocks always render; `$…$` inline is enabled because the
 *  models here emit inline math far more often than they write prices — with it
 *  off every `$x^2$` stayed literal (the reported symptom). */
const streamdownMath = createMathPlugin({ singleDollarTextMath: true });

/** Mermaid is the one plugin worth withholding: its engine is ~1 MB of diagram
 *  code for fences most answers never contain. It is fetched on idle the first
 *  time a fence looks like a diagram, and streamdown's block memo compares
 *  `plugins`, so a diagram that rendered as a code block upgrades in place the
 *  moment the chunk lands. */
const MERMAID_FENCE = /^[ \t]*(?:```|~~~)[ \t]*mermaid\b/m;
let mermaidPluginPromise: Promise<DiagramPlugin> | null = null;
function loadMermaidPlugin(): Promise<DiagramPlugin> {
  // Theme baked in at plugin creation. Passing it as the `mermaid` prop instead
  // made the plugin re-`initialize` mermaid's global config on every diagram
  // render (`getMermaid(config)` calls `initialize`), which is exactly the kind
  // of shared-state churn a second render (the fullscreen stage) does not need.
  //
  // mermaid's own default palette is pastel (pale yellow/blue boxes) and clashes
  // with the app's neutral surfaces; `neutral` is grayscale, which also survives
  // the dark card because the diagram surface is pinned light.
  mermaidPluginPromise ??= import("@streamdown/mermaid").then((m) =>
    m.createMermaidPlugin({ config: { theme: "neutral" } }),
  );
  return mermaidPluginPromise;
}

/** The streaming fade (inert unless `isAnimating`). `blurIn` is the docs' pick
 *  for fast models — blur hides batch arrivals better than plain opacity. `sep`
 *  stays at the default `"word"`: `"char"` was measured on this app's stream
 *  shape and is not affordable (a 3.2k-char block already cost **16.4ms per
 *  commit** against a 16.7ms frame budget, with 2.1k spans and 1.7k of them
 *  queued >400ms animating *after* their text had arrived). `maxBacklogMs: 120`
 *  is what keeps a word-level fade in step with a fast model: at 16k chars,
 *  9.2ms avg / 16.3ms worst with **no** span left animating more than 400ms late
 *  (the default backlog budget left 558 of them). Spans are stripped the moment
 *  streaming ends, so a settled block pays nothing.
 *
 *  `streamFx` decides which surface may pay for it, because the fade is only
 *  affordable where the reader is watching text arrive. A reasoning trace
 *  re-parses on every delta inside a clipped, collapsible panel; animated, the
 *  same 16k chars cost 18ms per commit over 12k DOM nodes (vs 7ms / 1.3k nodes)
 *  and 8.8k spans queued behind the stream. No caret: the block cursor at the
 *  end of the stream read as a stray glyph rather than a cursor. */
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
