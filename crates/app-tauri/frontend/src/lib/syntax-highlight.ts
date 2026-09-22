// Syntax Highlighting Engine using PrismJS for the Husk web frontend

import Prism from "prismjs";

// Import base grammars in topological dependency order
import "prismjs/components/prism-clike";
import "prismjs/components/prism-javascript";
import "prismjs/components/prism-typescript";
import "prismjs/components/prism-jsx";
import "prismjs/components/prism-tsx";

import "prismjs/components/prism-rust";
import "prismjs/components/prism-python";
import "prismjs/components/prism-go";
import "prismjs/components/prism-bash";
import "prismjs/components/prism-json";
import "prismjs/components/prism-toml";
import "prismjs/components/prism-yaml";
import "prismjs/components/prism-markdown";
import "prismjs/components/prism-sql";
import "prismjs/components/prism-c";
import "prismjs/components/prism-cpp";
import "prismjs/components/prism-csharp";
import "prismjs/components/prism-java";
import "prismjs/components/prism-kotlin";
import "prismjs/components/prism-swift";
import "prismjs/components/prism-css";
import "prismjs/components/prism-scss";
import "prismjs/components/prism-docker";
import "prismjs/components/prism-markup";
import "prismjs/components/prism-diff";
import "prismjs/components/prism-graphql";
import "prismjs/components/prism-protobuf";
import "prismjs/components/prism-ini";
import "prismjs/components/prism-ruby";

export interface HighlightToken {
  bgColor?: string;
  color?: string;
  content: string;
  htmlAttrs?: Record<string, string>;
  htmlStyle?: Record<string, string>;
  offset?: number;
}

export interface HighlightResult {
  bg?: string;
  fg?: string;
  rootStyle?: string | false;
  tokens: HighlightToken[][];
}

export interface StreamdownCodePlugin {
  getSupportedLanguages(): string[];
  getThemes(): [string, string];
  highlight(
    options: { code: string; language: string; themes?: [any, any] },
    callback?: (result: HighlightResult) => void
  ): HighlightResult | null;
  name: "shiki";
  supportsLanguage(language: string): boolean;
  type: "code-highlighter";
}

const LANGUAGE_ALIASES: Record<string, string> = {
  ts: "typescript",
  typescript: "typescript",
  tsx: "tsx",
  js: "javascript",
  javascript: "javascript",
  jsx: "jsx",
  mjs: "javascript",
  cjs: "javascript",
  rs: "rust",
  rust: "rust",
  py: "python",
  python: "python",
  go: "go",
  golang: "go",
  sh: "bash",
  bash: "bash",
  zsh: "bash",
  shell: "bash",
  json: "json",
  jsonc: "json",
  toml: "toml",
  yaml: "yaml",
  yml: "yaml",
  md: "markdown",
  markdown: "markdown",
  sql: "sql",
  c: "c",
  h: "c",
  cpp: "cpp",
  hpp: "cpp",
  cc: "cpp",
  cxx: "cpp",
  cs: "csharp",
  csharp: "csharp",
  java: "java",
  kt: "kotlin",
  kts: "kotlin",
  kotlin: "kotlin",
  swift: "swift",
  css: "css",
  scss: "scss",
  sass: "scss",
  html: "markup",
  xml: "markup",
  svg: "markup",
  docker: "docker",
  dockerfile: "docker",
  diff: "diff",
  patch: "diff",
  graphql: "graphql",
  gql: "graphql",
  proto: "protobuf",
  protobuf: "protobuf",
  ini: "ini",
  env: "bash",
};

export function normalizeLanguage(lang?: string): string {
  if (!lang) return "text";
  const clean = lang.toLowerCase().trim();
  return LANGUAGE_ALIASES[clean] || clean;
}

export function getPrismGrammar(lang: string): Prism.Grammar | null {
  const normalized = normalizeLanguage(lang);
  if (normalized === "text" || normalized === "plain") return null;
  return Prism.languages[normalized] || null;
}

// GitHub Dark / Modern Code Palette
export function getTokenColor(type: string): string {
  switch (type) {
    case "comment":
    case "prolog":
    case "doctype":
    case "cdata":
      return "#8b949e"; // Muted gray
    case "keyword":
    case "builtin":
      return "#ff7b72"; // Vibrant coral red
    case "tag":
      return "#7ee787"; // Emerald green for HTML/JSX tags
    case "string":
    case "char":
    case "attr-value":
      return "#a5d6ff"; // Ice blue for strings
    case "function":
    case "function-variable":
      return "#d2a8ff"; // Soft purple for functions
    case "number":
    case "boolean":
      return "#79c0ff"; // Sky blue
    case "class-name":
    case "maybe-class-name":
    case "type":
      return "#ffa657"; // Warm amber for types/classes
    case "operator":
      return "#ff7b72"; // Operators
    case "punctuation":
      return "#8b949e"; // Subtle punctuation
    case "attr-name":
    case "property":
      return "#79c0ff"; // Attributes / properties
    case "regex":
    case "important":
      return "#f0883e"; // Orange
    case "variable":
    case "constant":
      return "#79c0ff";
    case "deleted":
      return "#ffa198"; // Diff red
    case "inserted":
      return "#56d364"; // Diff green
    default:
      return "#e4e4e7"; // Default text color
  }
}

/**
 * Resolve token type and aliases to a color
 */
export function resolveTokenColor(type?: string, alias?: string | string[]): string {
  if (type) {
    const c = getTokenColor(type);
    if (c !== "#e4e4e7") return c;
  }
  if (alias) {
    if (typeof alias === "string") {
      const c = getTokenColor(alias);
      if (c !== "#e4e4e7") return c;
    } else if (Array.isArray(alias)) {
      for (const a of alias) {
        const c = getTokenColor(a);
        if (c !== "#e4e4e7") return c;
      }
    }
  }
  return type ? getTokenColor(type) : "#e4e4e7";
}

/**
 * Highlight cache — Prism tokenizes from scratch, and the streaming tail's text
 * changes on every delta while unrelated re-renders (usage tick, `expanded` toggle)
 * used to build a fresh result object and break the code block's memo. Caching by
 * content returns the identical object for identical input, so the memo holds.
 *
 * Bounded by entries and total lines, since one entry can be a whole file. Callers
 * treat results as immutable — the streaming plugin copies before appending.
 */
const CACHE_MAX_ENTRIES = 32;
const CACHE_MAX_LINES = 20_000;
const cache = new Map<string, { value: unknown; lines: number }>();
let cachedLines = 0;

function cacheKey(mode: string, code: string, language: string): string {
  return `${mode}\u0000${language}\u0000${code}`;
}

/** Newline count without materializing the split array. */
function countLines(code: string): number {
  let n = 1;
  for (let i = 0; i < code.length; i++) {
    if (code.charCodeAt(i) === 10) n += 1;
  }
  return n;
}

function cacheGet<T>(key: string): T | undefined {
  const hit = cache.get(key);
  if (!hit) return undefined;
  // LRU touch — re-insert so the Map's insertion order stays recency-ordered.
  cache.delete(key);
  cache.set(key, hit);
  return hit.value as T;
}

function cachePut<T>(key: string, value: T, lines: number): T {
  const previous = cache.get(key);
  if (previous) cachedLines -= previous.lines;
  cache.set(key, { value, lines });
  cachedLines += lines;
  while ((cache.size > CACHE_MAX_ENTRIES || cachedLines > CACHE_MAX_LINES) && cache.size > 1) {
    const oldest = cache.keys().next().value as string | undefined;
    if (oldest === undefined) break;
    cachedLines -= cache.get(oldest)?.lines ?? 0;
    cache.delete(oldest);
  }
  return value;
}

/** A single unstyled token line — the "no grammar / no highlight" shape the
 * plugin contract expects. */
function plainLine(content: string): HighlightToken[] {
  return [
    {
      content,
      color: "#e4e4e7",
      bgColor: "transparent",
      htmlStyle: {
        color: "#e4e4e7",
        "--sdm-c": "#e4e4e7",
        "--shiki-dark": "#e4e4e7",
      },
      offset: 0,
    },
  ];
}

export function highlightCodeWithPrism(code: string, language: string): HighlightResult {
  const key = cacheKey("tokens", code, language);
  const hit = cacheGet<HighlightResult>(key);
  if (hit) return hit;
  const result = tokenizeWithoutCache(code, language);
  return cachePut(key, result, result.tokens.length);
}

/**
 * Tokenize pure code into HighlightResult (lines of tokens) compatible with
 * Streamdown — the uncached worker behind `highlightCodeWithPrism`.
 */
function tokenizeWithoutCache(code: string, language: string): HighlightResult {
  const grammar = getPrismGrammar(language);
  if (!grammar) {
    const lines = code.split("\n");
    return {
      bg: "#141416",
      fg: "#e4e4e7",
      tokens: lines.map((line) => plainLine(line || "")),
    };
  }

  const prismTokens = Prism.tokenize(code, grammar);
  const lines: HighlightToken[][] = [[]];

  const addToken = (content: string, type?: string) => {
    const parts = content.split("\n");
    for (let i = 0; i < parts.length; i++) {
      if (i > 0) {
        lines.push([]);
      }
      if (parts[i].length > 0) {
        const color = type ? getTokenColor(type) : "#e4e4e7";
        lines[lines.length - 1].push({
          content: parts[i],
          color,
          bgColor: "transparent",
          htmlStyle: {
            color,
            "--sdm-c": color,
            "--shiki-dark": color,
          },
          htmlAttrs: type ? { className: `token ${type}`, "data-token": type } : undefined,
          offset: 0,
        });
      }
    }
  };

  const walk = (token: string | Prism.Token, parentType?: string) => {
    if (typeof token === "string") {
      addToken(token, parentType);
    } else if (typeof token.content === "string") {
      const type = token.type || (typeof token.alias === "string" ? token.alias : parentType);
      addToken(token.content, type);
    } else if (Array.isArray(token.content)) {
      const type = token.type || (typeof token.alias === "string" ? token.alias : parentType);
      for (const sub of token.content) {
        if (typeof sub === "string") {
          addToken(sub, type);
        } else {
          walk(sub, type);
        }
      }
    } else if (typeof token.content === "object" && token.content) {
      const type = token.type || (typeof token.alias === "string" ? token.alias : parentType);
      walk(token.content as Prism.Token, type);
    }
  };

  for (const token of prismTokens) {
    walk(token);
  }

  // Ensure every line has at least one token
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].length === 0) {
      lines[i].push(...plainLine(""));
    }
  }

  return {
    bg: "#141416",
    fg: "#e4e4e7",
    tokens: lines,
  };
}

/**
 * Return highlighted HTML string for direct rendering
 */
export function highlightCodeToHtml(code: string, language: string): string {
  const key = cacheKey("html", code, language);
  const hit = cacheGet<string>(key);
  if (hit !== undefined) return hit;
  const grammar = getPrismGrammar(language);
  if (!grammar) {
    return escapeHtml(code);
  }
  const norm = normalizeLanguage(language);
  try {
    const html = Prism.highlight(code, grammar, norm);
    return cachePut(key, html, countLines(code));
  } catch {
    return escapeHtml(code);
  }
}

function escapeHtml(str: string): string {
  return str
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}


/**
 * Streamdown-compatible code highlighter plugin using PrismJS
 */
export const prismCodePlugin: StreamdownCodePlugin = {
  name: "shiki",
  type: "code-highlighter",
  getSupportedLanguages() {
    return Object.keys(Prism.languages);
  },
  getThemes() {
    return ["github-dark", "github-dark"];
  },
  supportsLanguage(language: string) {
    return Boolean(getPrismGrammar(language));
  },
  highlight(options, callback) {
    const result = highlightCodeWithPrism(options.code, options.language);
    if (callback) {
      callback(result);
    }
    return result;
  },
};

/**
 * Streaming variant — only for the block that is still arriving (`isAnimating`).
 * Its last line is incomplete, and a half-typed string or comment re-tokenizes
 * differently on every delta, so that line's colors flicker and the work is redone
 * for text that is about to change anyway.
 *
 * So the complete-line prefix is highlighted through the shared cache and the trailing
 * partial line renders as plain text; the finished block switches back for its exact
 * final highlight in one pass.
 */
export const prismCodePluginStreaming: StreamdownCodePlugin = {
  name: "shiki",
  type: "code-highlighter",
  getSupportedLanguages() {
    return Object.keys(Prism.languages);
  },
  getThemes() {
    return ["github-dark", "github-dark"];
  },
  supportsLanguage(language: string) {
    return Boolean(getPrismGrammar(language));
  },
  highlight(options, callback) {
    const { code, language } = options;
    const cut = code.lastIndexOf("\n");
    let result: HighlightResult;
    if (cut < 0) {
      // Single unterminated line — nothing complete to tokenize yet.
      result = { bg: "#141416", fg: "#e4e4e7", tokens: [plainLine(code)] };
    } else {
      const head = highlightCodeWithPrism(code.slice(0, cut + 1), language);
      const tail = code.slice(cut + 1);
      // Copy before appending: `head` may be a cache entry, and callers must
      // never see a mutated cache value.
      const tokens = tail ? [...head.tokens, plainLine(tail)] : head.tokens;
      result = { ...head, tokens };
    }
    if (callback) {
      callback(result);
    }
    return result;
  },
};
