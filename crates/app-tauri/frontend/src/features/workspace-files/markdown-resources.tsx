import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Streamdown, defaultRehypePlugins } from "streamdown";
import { openUrl } from "@/lib/agent-ipc/sessions";
import { codeHighlightPlugin } from "@/lib/syntax-highlight";
import { parseMarkdownResourceReference, workspaceFileHref, type MarkdownResourceTarget } from "./file-reference";
import type { FilePreview } from "./types";

interface ResourceNode {
  type?: string;
  value?: string;
  tagName?: string;
  properties?: Record<string, unknown>;
  children?: ResourceNode[];
}

function markdownResources({ root, documentPath }: { root: string; documentPath: string }) {
  return (tree: ResourceNode) => {
    const visit = (node: ResourceNode) => {
      const attribute = node.tagName === "a" ? "href" : node.tagName === "img" ? "src" : undefined;
      if (attribute && typeof node.properties?.[attribute] === "string") {
        const value = node.properties[attribute] as string;
        const target = parseMarkdownResourceReference(value, root, documentPath);
        if (target) node.properties[attribute] = `${workspaceFileHref(target, root)}${target.fragment === undefined ? "" : `#${encodeURIComponent(target.fragment)}`}`;
        else if (!/^https?:\/\//i.test(value)) {
          const label = attribute === "src" ? [{ type: "text", value: `图片：${node.properties.alt || "图片"}` }] : node.children ?? [];
          const reason = value.split("#", 1)[0].includes("?") ? "本地资源不支持查询参数" : "资源地址超出工作区或不受支持";
          node.tagName = "span";
          node.properties = {};
          node.children = [...label, { type: "text", value: `（${reason}）` }];
        }
      }
      node.children?.forEach(visit);
    };
    visit(tree);
  };
}

type ImageProps = Pick<React.ComponentProps<"img">, "alt" | "title" | "width" | "height">;

function imageDimension(value: number | string | undefined): number | undefined {
  if (typeof value === "string" && !/^[1-9]\d*$/.test(value)) return undefined;
  const dimension = Number(value);
  return Number.isSafeInteger(dimension) && dimension > 0 ? dimension : undefined;
}

function remoteImageSource(src: string): string | undefined {
  if (!/^https:\/\//i.test(src)) return undefined;
  try {
    const url = new URL(src);
    if (url.protocol === "https:" && !url.username && !url.password) return url.href;
  } catch {
    return undefined;
  }
  return undefined;
}

function RemoteMarkdownImage({ src, alt, title, width, height }: { src: string } & ImageProps) {
  const [failed, setFailed] = useState(false);
  if (failed) return <span className="workspace-files-note" role="alert" title={title}>
    {alt ? `图片：${alt} · ` : "图片 · "}远程图片加载失败
  </span>;
  return <img src={src} alt={alt ?? ""} title={title} width={width} height={height}
    referrerPolicy="no-referrer" onError={() => setFailed(true)} />;
}

function MarkdownImage({ root, path, alt, title, width, height }: { root: string; path: string } & ImageProps) {
  const [result, setResult] = useState<{ src?: string; error?: string }>({});
  useEffect(() => {
    let alive = true;
    void invoke<FilePreview>("workspace_preview_file", { root, path }).then(file => {
      if (!alive) return;
      if (file.kind === "image" && file.data && /^image\/(png|jpeg|gif|webp)$/.test(file.mime ?? "")) {
        setResult({ src: `data:${file.mime};base64,${file.data}` });
      } else {
        setResult({ error: file.kind === "too_large" ? "图片过大，无法预览（上限 8 MiB）" : "不支持的图片格式（仅支持 PNG、JPEG、GIF、WebP）" });
      }
    }).catch(error => {
      if (alive) setResult({ error: String(error) });
    });
    return () => { alive = false; };
  }, [root, path]);
  if (result.src) return <img src={result.src} alt={alt ?? ""} title={title} width={width} height={height} onError={() => setResult({ error: "图片数据无法解码" })} />;
  return <span className="workspace-files-note" role={result.error ? "alert" : "status"}>
    {alt ? `图片：${alt} · ` : "图片 · "}{result.error ?? "正在读取图片"}
  </span>;
}

export function WorkspaceMarkdown({ text, root, documentPath, onNavigate }: {
  text: string;
  root: string;
  documentPath: string;
  onNavigate: (target: MarkdownResourceTarget) => void;
}) {
  const navigate = useRef(onNavigate);
  navigate.current = onNavigate;
  const rehypePlugins = useMemo<React.ComponentProps<typeof Streamdown>["rehypePlugins"]>(() => [
    defaultRehypePlugins.raw, [markdownResources, { root, documentPath }],
    defaultRehypePlugins.sanitize, defaultRehypePlugins.harden,
  ], [root, documentPath]);
  const components = useMemo(() => ({
    a: ({ href, children }: React.ComponentProps<"a">) => {
      const target = href ? parseMarkdownResourceReference(href, root, documentPath) : null;
      if (target) return <a href={href} onClick={event => { event.preventDefault(); navigate.current(target); }}>{children}</a>;
      if (/^https?:\/\//i.test(href ?? "")) return <a href={href} onClick={event => {
        event.preventDefault();
        void openUrl(href!).catch(() => {});
      }}>{children}</a>;
      return <span title={href}>{children}</span>;
    },
    img: ({ src, alt, title, width, height }: React.ComponentProps<"img">) => {
      const dimensions = { width: imageDimension(width), height: imageDimension(height) };
      const target = src ? parseMarkdownResourceReference(src, root, documentPath) : null;
      if (target) return <MarkdownImage key={`${root}\0${documentPath}\0${target.path}`} root={root} path={target.path} alt={alt} title={title} {...dimensions} />;
      const remote = typeof src === "string" ? remoteImageSource(src) : undefined;
      return remote ? <RemoteMarkdownImage key={remote} src={remote} alt={alt} title={title} {...dimensions} />
        : <span className="workspace-files-note">{alt ? `图片：${alt}` : "图片"}（仅预览工作区内支持的图片或无凭据的 HTTPS 图片）</span>;
    },
  }), [root, documentPath]);
  return <Streamdown mode="static" skipHtml parseIncompleteMarkdown={false} isAnimating={false}
    plugins={{ code: codeHighlightPlugin }} controls={false} rehypePlugins={rehypePlugins} components={components}>
    {text}
  </Streamdown>;
}
