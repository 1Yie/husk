import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Streamdown } from "streamdown";
import { openUrl } from "@/lib/agent-ipc/sessions";
import { ArrowLeft, ChevronDown, ChevronRight, Folder, Link, Maximize2, RefreshCw } from "lucide-react";
import { WorkspaceFileIcon } from "./file-icon";
import { CodeLoadingSkeleton, DirectoryLoadingSkeleton } from "./loading";
import { Dialog, DialogContent, DialogTitle, DialogTrigger } from "@/components/ui/dialog";
import { codeHighlightPlugin, normalizeLanguage, type HighlightResult, type HighlightToken } from "@/lib/syntax-highlight";
import type { DirectoryListing, FilePreview, WorkspaceEntry } from "./types";
import "./workspace-files.css";

export interface WorkspaceFilesProps {
  root: string;
  onOpenWorkspace?: () => void;
  onBack?: () => void;
}

type DirectoryState = { listing?: DirectoryListing; error?: string; loading?: boolean };
type PreviewState = { file?: FilePreview; path?: string; error?: string; loading?: boolean };

function sizeLabel(size: number) {
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KiB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MiB`;
}

function languageOf(name: string) {
  const extension = name.split(".").pop()?.toLowerCase() ?? "";
  return normalizeLanguage(extension === "md" ? "markdown" : extension);
}

function PreviewStatus({ file, language, lines }: { file: FilePreview; language?: string; lines?: number }) {
  return (
    <footer className="workspace-files-status" aria-label="文件状态">
      <span>{language || file.mime || "文件"}</span>
      {lines !== undefined && <span>{lines.toLocaleString()} 行</span>}
      <span>{sizeLabel(file.size)}</span>
      <span className="workspace-files-readonly">只读</span>
    </footer>
  );
}

function SourcePreview({ file, large }: { file: FilePreview; large: boolean }) {
  const source = file.text ?? "";
  // Bound DOM/tokenization cost independently of the backend's byte limit.
  const cap = large ? 1_000_000 : 200_000;
  const lineCap = large ? 10_000 : 2_000;
  const text = useMemo(() => source.slice(0, cap).split("\n").slice(0, lineCap).join("\n"), [source, cap, lineCap]);
  const language = languageOf(file.name);
  const [highlight, setHighlight] = useState<{ code: string; language: string; result: HighlightResult | null } | null>(null);
  useEffect(() => {
    let live = true;
    let settled = false;
    let fallback: ReturnType<typeof setTimeout> | undefined;
    setHighlight(null);
    const publish = (result: HighlightResult | null) => {
      if (!live) return;
      settled = true;
      if (fallback !== undefined) clearTimeout(fallback);
      setHighlight({ code: text, language, result });
    };
    try {
      const initial = codeHighlightPlugin.highlight({ code: text, language }, publish);
      // A lazy grammar failing to load must never leave an endless skeleton.
      // Plain text is readable immediately after this bounded upgrade window.
      if (!settled) fallback = setTimeout(() => publish(initial), 180);
    } catch {
      publish(null);
    }
    return () => {
      live = false;
      if (fallback !== undefined) clearTimeout(fallback);
    };
  }, [text, language]);
  const ready = highlight?.code === text && highlight.language === language ? highlight : null;
  const lines: HighlightToken[][] = ready?.result?.tokens ?? text.split("\n").map(content => [{ content }]);
  return <>
    {text.length < source.length && <p className="workspace-files-note">为保持流畅，仅显示前 {lineCap.toLocaleString()} 行 / {cap.toLocaleString()} 字符。{!large && "展开预览可阅读更多内容。"}</p>}
    {ready ? <div className="workspace-files-code-scroll" tabIndex={0} aria-label="文件源码"
      style={{ "--workspace-gutter": `calc(${Math.max(3, String(lines.length).length)}ch + 20px)` } as CSSProperties}>
      <pre><code>{lines.map((line, index) => <span className="workspace-files-code-line" key={index}>
        <span className="workspace-files-line-number" aria-hidden="true">{index + 1}</span>
        <span>{line.map((token, i) => (
          // The shared chat highlighter intentionally emits dark-only inline
          // colors. Preview uses semantic token classes and its own light/dark
          // editor palette; unclassified tokens inherit readable foreground.
          <span key={i} className={token.htmlAttrs?.className}>{token.content}</span>
        ))}{"\n"}</span>
      </span>)}</code></pre>
    </div> : <CodeLoadingSkeleton />}
    <PreviewStatus file={file} language={language || "text"} lines={source.split("\n").length} />
  </>;
}

function PreviewBody({ file, large = false }: { file: FilePreview; large?: boolean }) {
  if (file.kind === "text") return <TextPreview file={file} large={large} />;
  // Only signature-verified raster MIME types from the read-only command are allowed.
  if (file.kind === "image" && file.data && /^image\/(png|jpeg|gif|webp)$/.test(file.mime ?? "")) {
    return <>
      <div className="workspace-files-image"><img alt={file.name} src={`data:${file.mime};base64,${file.data}`} /></div>
      <PreviewStatus file={file} />
    </>;
  }
  return <p className="workspace-files-note">
    {file.kind === "too_large" ? "文件过大，无法预览。文本上限 2 MiB，图片上限 8 MiB。" : "二进制或不支持的文件，无法预览。"}
    <br />{file.name} · {sizeLabel(file.size)}
  </p>;
}

function TextPreview({ file, large = false }: { file: FilePreview; large?: boolean }) {
  const [source, setSource] = useState(false);
  if (!/\.(md|markdown)$/i.test(file.name)) return <SourcePreview file={file} large={large} />;
  const text = file.text ?? "";
  const rendered = text.slice(0, 200_000);
  return (
    <div className="workspace-files-text-preview">
      <div role="tablist" aria-label="Markdown 显示模式" className="workspace-files-preview-modes">
        <button type="button" role="tab" aria-selected={!source} onClick={() => setSource(false)}>预览</button>
        <button type="button" role="tab" aria-selected={source} onClick={() => setSource(true)}>源码</button>
      </div>
      {source ? <SourcePreview file={file} large={large} /> : (
        <div className="workspace-files-markdown" aria-label="Markdown 预览">
          <Streamdown mode="static" skipHtml parseIncompleteMarkdown={false} isAnimating={false}
            plugins={{ code: codeHighlightPlugin }} controls={false}
            components={{
              a: ({ href, children }) => /^https?:\/\//i.test(href ?? "") ? (
                <a href={href} onClick={(event) => {
                  event.preventDefault();
                  void openUrl(href!).catch(() => {});
                }}>{children}</a>
              ) : <span title={href}>{children}</span>,
              img: ({ alt }) => <span className="workspace-files-note">{alt ? `图片：${alt}` : "图片"}（请从文件树预览）</span>,
            }}>
            {rendered}
          </Streamdown>
          {rendered.length < text.length && <p className="workspace-files-note">Markdown 预览仅渲染前 200,000 字符；源码模式可查看有界原文。</p>}
        </div>
      )}
      {!source && <PreviewStatus file={file} language="Markdown" lines={text.split("\n").length} />}
    </div>
  );
}

export function WorkspaceFiles({ root, onOpenWorkspace, onBack }: WorkspaceFilesProps) {
  if (!root.trim()) return <section className="workspace-files workspace-files-empty" aria-label="预览工作区文件">
    <p>尚未打开工作区</p>
    {onOpenWorkspace && <button onClick={onOpenWorkspace}>打开工作区</button>}
    {onBack && <button onClick={onBack}>返回侧栏</button>}
  </section>;
  // A workspace switch must not retain paths, cached listings, dialogs or pending requests.
  return <WorkspaceBrowser key={root} root={root} onBack={onBack} />;
}

function WorkspaceBrowser({ root, onBack }: { root: string; onBack?: () => void }) {
  const [directories, setDirectories] = useState<Record<string, DirectoryState>>({});
  const [expanded, setExpanded] = useState<Set<string>>(new Set([""]));
  const [preview, setPreview] = useState<PreviewState>({});
  const [dialogOpen, setDialogOpen] = useState(false);
  const generation = useRef(0);
  const previewRequest = useRef(0);
  const requested = useRef(new Set<string>());
  const tree = useRef<HTMLDivElement>(null);
  const treeScroll = useRef(0);
  const lastOpened = useRef("");
  const restoreFocus = useRef(false);

  useLayoutEffect(() => {
    if (preview.path || !tree.current) return;
    tree.current.scrollTop = treeScroll.current;
    if (restoreFocus.current) {
      Array.from(tree.current.querySelectorAll<HTMLButtonElement>("[data-workspace-path]"))
        .find((button) => button.dataset.workspacePath === lastOpened.current)
        ?.focus({ preventScroll: true });
      restoreFocus.current = false;
    }
  }, [preview.path]);

  function loadDirectory(path: string) {
    if (requested.current.has(path)) return;
    requested.current.add(path);
    const epoch = generation.current;
    setDirectories(current => ({ ...current, [path]: { loading: true } }));
    void invoke<DirectoryListing>("workspace_list_files", { root, path }).then(listing => {
      if (epoch === generation.current) setDirectories(current => ({ ...current, [path]: { listing } }));
    }).catch(error => {
      if (epoch === generation.current) {
        requested.current.delete(path);
        setDirectories(current => ({ ...current, [path]: { error: String(error) } }));
      }
    });
  }

  useEffect(() => {
    loadDirectory("");
    return () => {
      generation.current += 1;
      previewRequest.current += 1;
      requested.current.clear();
    };
  }, [root]);

  function refresh() {
    generation.current += 1;
    previewRequest.current += 1;
    requested.current.clear();
    setDirectories({});
    setExpanded(new Set([""]));
    setPreview({});
    setDialogOpen(false);
    treeScroll.current = 0;
    lastOpened.current = "";
    loadDirectory("");
  }

  function openFile(entry: WorkspaceEntry) {
    treeScroll.current = tree.current?.scrollTop ?? treeScroll.current;
    lastOpened.current = entry.path;
    const id = ++previewRequest.current;
    const epoch = generation.current;
    setPreview({ path: entry.path, loading: true });
    void invoke<FilePreview>("workspace_preview_file", { root, path: entry.path }).then(file => {
      if (id === previewRequest.current && epoch === generation.current) setPreview({ path: entry.path, file });
    }).catch(error => {
      if (id === previewRequest.current && epoch === generation.current) setPreview({ path: entry.path, error: String(error) });
    });
  }

  function backToFiles() {
    previewRequest.current += 1;
    restoreFocus.current = true;
    setDialogOpen(false);
    setPreview({});
  }

  function toggleDirectory(path: string) {
    setExpanded(current => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
    if (!expanded.has(path)) loadDirectory(path);
  }

  function renderDirectory(path: string, depth: number): React.ReactNode {
    const state = directories[path];
    if (!state || state.loading) return <DirectoryLoadingSkeleton depth={depth} />;
    if (state.error) return <div className="workspace-files-note" role="alert">
      {state.error}<button onClick={() => loadDirectory(path)}>重试</button>
    </div>;
    const listing = state.listing!;
    return <>
      {listing.entries.length === 0 && <p className="workspace-files-note">此目录为空</p>}
      <ul className="workspace-files-list">
        {listing.entries.map(entry => {
          const directory = entry.kind === "directory";
          const isOpen = expanded.has(entry.path);
          return <li key={entry.path}>
            <button className="workspace-files-entry" style={{ paddingLeft: 8 + depth * 14 }}
              data-workspace-path={entry.path}
              aria-label={entry.kind === "symlink" ? `${entry.name}（符号链接）` : entry.name}
              title={entry.path} aria-expanded={directory ? isOpen : undefined}
              aria-current={!directory && (preview.path ?? lastOpened.current) === entry.path ? "true" : undefined}
              onClick={() => directory ? toggleDirectory(entry.path) : openFile(entry)}>
              {directory ? (isOpen ? <ChevronDown /> : <ChevronRight />) : <span className="workspace-files-chevron-space" />}
              {directory ? <Folder /> : entry.kind === "symlink" ? <Link /> : <WorkspaceFileIcon name={entry.name} />}
              <span className="workspace-files-name">{entry.name}</span>
              {entry.kind === "symlink" && <span className="workspace-files-link-label">链接</span>}
            </button>
            {directory && isOpen && renderDirectory(entry.path, depth + 1)}
          </li>;
        })}
      </ul>
      {listing.truncated && <p className="workspace-files-note">目录内容过多，仅显示前 2,000 项。</p>}
    </>;
  }

  return <section className="workspace-files" aria-label="预览工作区文件" data-view={preview.path ? "preview" : "tree"}>
    {!preview.path && <header className="workspace-files-header">
      {onBack && <button type="button" onClick={onBack} aria-label="返回侧栏" title="返回侧栏">
        <ArrowLeft size={14} aria-hidden="true" />
      </button>}
      <span className="workspace-files-name" title={root}>{root.split(/[\\/]/).filter(Boolean).pop() ?? root}</span>
      <button onClick={refresh} title="刷新文件" aria-label="刷新文件"><RefreshCw size={14} /></button>
    </header>}
    {!preview.path ? (
      <div ref={tree} className="workspace-files-tree" aria-label="工作区目录">{renderDirectory("", 0)}</div>
    ) : (
    <section className="workspace-files-preview" aria-label="文件预览">
      <header className="workspace-files-header workspace-files-editor-header">
        <button type="button" onClick={backToFiles} aria-label="返回文件列表" title="返回文件列表">
          <ArrowLeft size={15} aria-hidden="true" />
        </button>
        <WorkspaceFileIcon name={preview.path} className="workspace-files-editor-icon" />
        <span className="workspace-files-name" title={preview.path}>
          {preview.path.split("/").pop()}
        </span>
        {preview.file && <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
          <DialogTrigger asChild><button title="展开预览" aria-label="展开预览"><Maximize2 size={14} /></button></DialogTrigger>
          <DialogContent className="workspace-files-dialog" aria-describedby={undefined}>
            <DialogTitle className="workspace-files-dialog-title" title={preview.path}>{preview.path}</DialogTitle>
            {dialogOpen && <PreviewBody file={preview.file} large />}
          </DialogContent>
        </Dialog>}
      </header>
      <div className="workspace-files-breadcrumbs" aria-label="文件路径" title={preview.path}>
        {preview.path.split("/").join(" › ")}
      </div>
      {preview.loading && <CodeLoadingSkeleton label="正在读取文件" />}
      {preview.error && <p className="workspace-files-note" role="alert">{preview.error}</p>}
      {preview.file && <PreviewBody file={preview.file} />}
    </section>
    )}
  </section>;
}
