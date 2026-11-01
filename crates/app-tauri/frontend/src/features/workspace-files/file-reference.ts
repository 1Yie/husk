export interface WorkspaceFileTarget {
  path: string;
  line?: number;
  column?: number;
}

export interface MarkdownResourceTarget extends WorkspaceFileTarget {
  fragment?: string;
}

export interface WorkspaceFileRequest extends WorkspaceFileTarget {
  workspaceRoot: string;
  sessionId: number;
  sessionKey?: string;
  activation: number;
}

/** Resolve a local reference against the trusted workspace, never against the page URL. */
export function parseWorkspaceFileReference(href: string, workspaceRoot: string): WorkspaceFileTarget | null {
  return parseFileReference(href, workspaceRoot);
}

/** Resolve Markdown resources relative to the trusted document's directory. */
export function parseMarkdownResourceReference(href: string, workspaceRoot: string, documentPath: string): MarkdownResourceTarget | null {
  if (!documentPath || documentPath.startsWith("/") || documentPath.split(/[\\/]/).some(part => !part || part === "." || part === ".." || part.includes(":"))) return null;
  const hash = href.indexOf("#");
  if (hash === 0 && /^#L\d+(?:C\d+)?$/i.test(href)) return parseFileReference(`${workspaceFileHref({ path: documentPath }, workspaceRoot)}${href}`, workspaceRoot);
  if (hash < 0 || /^#L\d+(?:C\d+)?$/i.test(href.slice(hash))) return parseFileReference(href, workspaceRoot, documentPath);
  let fragment: string;
  try {
    fragment = decodeURIComponent(href.slice(hash + 1));
  } catch {
    return null;
  }
  if (/[\u0000-\u001f\u007f]/.test(fragment)) return null;
  const reference = href.slice(0, hash);
  const target = reference
    ? parseFileReference(reference, workspaceRoot, documentPath)
    : parseWorkspaceFileReference(workspaceFileHref({ path: documentPath }, workspaceRoot), workspaceRoot);
  return target && target.line === undefined ? { ...target, fragment } : null;
}

function parseFileReference(href: string, workspaceRoot: string, documentPath?: string): WorkspaceFileTarget | null {
  if (!workspaceRoot || !href || href.startsWith("#") || /[\u0000-\u001f\u007f]/.test(href)) return null;
  let value = href;
  let line: number | undefined;
  let column: number | undefined;
  const fragment = value.match(/#L(\d+)(?:C(\d+))?$/i);
  if (fragment) {
    line = Number(fragment[1]);
    column = fragment[2] === undefined ? undefined : Number(fragment[2]);
    value = value.slice(0, fragment.index);
  } else if (value.includes("#")) return null;
  const position = value.match(/:(\d+)(?::(\d+))?$/);
  if (position) {
    if (fragment) return null;
    line = Number(position[1]);
    column = position[2] === undefined ? undefined : Number(position[2]);
    value = value.slice(0, position.index);
  }
  if ((line !== undefined && !Number.isSafeInteger(line)) || (column !== undefined && !Number.isSafeInteger(column))) return null;
  if ((line !== undefined && line < 1) || (column !== undefined && column < 1)) return null;
  if (!value) return null;
  if (/^file:\/\//i.test(value)) {
    const local = value.match(/^file:\/\/(localhost)?(\/.*)$/i);
    if (!local) return null;
    value = local[2];
    if (/^\/[a-z]:[\\/]/i.test(value)) value = value.slice(1);
  } else if (!/^[a-z]:[\\/]/i.test(value) && /^[a-z][a-z\d+.-]*:/i.test(value)) return null;
  try {
    value = decodeURIComponent(value);
  } catch {
    return null;
  }
  if (/[\u0000-\u001f\u007f?#]/.test(value) || /%(?:2e|2f|5c|25)/i.test(value)) return null;
  value = value.replace(/\\/g, "/");
  const root = workspaceRoot.replace(/\\/g, "/").replace(/\/+$/, "") || "/";
  if (/^[a-z]:/i.test(root) && /^\/[a-z]:\//i.test(value)) value = value.slice(1);
  if (!root.startsWith("/") && !/^[a-z]:(?:\/|$)/i.test(root)) return null;
  if (value.startsWith("//") || (!documentPath && value.split("/").includes(".."))) return null;
  const absolute = value.startsWith("/") || /^[a-z]:\//i.test(value);
  if (absolute) {
    const windows = /^[a-z]:/i.test(root);
    const candidate = windows ? value.toLowerCase() : value;
    const boundary = windows ? root.toLowerCase() : root;
    const prefix = boundary === "/" ? "/" : `${boundary}/`;
    if (!candidate.startsWith(prefix)) return null;
    value = value.slice(prefix.length);
  }
  const parts = documentPath && !absolute ? documentPath.replace(/\\/g, "/").split("/").slice(0, -1) : [];
  for (const part of value.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      if (!parts.length) return null;
      parts.pop();
    } else parts.push(part);
  }
  if (!parts.length || parts.some((part) => part.includes(":"))) return null;
  return { path: parts.join("/"), ...(line === undefined ? {} : { line }), ...(column === undefined ? {} : { column }) };
}

export function workspaceFileHref(target: WorkspaceFileTarget, workspaceRoot: string): string {
  const root = workspaceRoot.replace(/\\/g, "/").replace(/\/+$/, "");
  const path = `${root}/${target.path}`.split("/").map(encodeURIComponent).join("/");
  return `${path.startsWith("/") ? path : `/${path}`}${target.line === undefined ? "" : `#L${target.line}${target.column === undefined ? "" : `C${target.column}`}`}`;
}

interface LinkNode {
  type: string;
  tagName?: string;
  properties?: Record<string, unknown>;
  children?: LinkNode[];
}

/** Runs after raw HTML parsing and before the unchanged sanitize/harden plugins. */
export function workspaceFileLinks({ root }: { root: string }) {
  return (tree: LinkNode) => {
    const visit = (node: LinkNode) => {
      if (node.tagName === "a" && typeof node.properties?.href === "string") {
        const target = parseWorkspaceFileReference(node.properties.href, root);
        if (target) node.properties.href = workspaceFileHref(target, root);
      }
      node.children?.forEach(visit);
    };
    visit(tree);
  };
}
