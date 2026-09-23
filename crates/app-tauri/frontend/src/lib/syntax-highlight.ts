// Code highlighting — twinkleplop engine.
//
// Replaced PrismJS. Each language is a table-driven state machine
// (`@twinkleplop/<language>`) that scans the source with `charCodeAt` and
// returns a flat `Uint32Array` of `[type, start, end]` triplets, so
// re-tokenizing a streaming tail is a linear scan instead of a full grammar
// re-run over the block. Colours come from `@twinkleplop/theme-github`.
//
// Two consumers, one tokenizer:
//   * the Streamdown plugin (chat code blocks) wants per-line token arrays,
//   * `highlightCodeToHtml` (tool capsules, raw history) wants an HTML string.

import type { LanguageFactory, TokenizeResult } from "@twinkleplop/core";
import { dark as themeDark } from "@twinkleplop/theme-github/tokens";

import { tokenize as bashGrammar } from "@twinkleplop/bash";
import { tokenize as cssGrammar } from "@twinkleplop/css";
import { tokenize as goGrammar } from "@twinkleplop/go";
import { tokenize as htmlGrammar } from "@twinkleplop/html";
import { tokenize as javascriptGrammar } from "@twinkleplop/javascript";
import { tokenize as jsonGrammar } from "@twinkleplop/json";
import { tokenize as markdownGrammar } from "@twinkleplop/markdown";
import { tokenize as pythonGrammar } from "@twinkleplop/python";
import { tokenize as rustGrammar } from "@twinkleplop/rust";
import { tokenize as sqlGrammar } from "@twinkleplop/sql";
import { tokenize as tomlGrammar } from "@twinkleplop/toml";
import { tokenize as tsxGrammar } from "@twinkleplop/tsx";
import { tokenize as typescriptGrammar } from "@twinkleplop/typescript";
import { tokenize as yamlGrammar } from "@twinkleplop/yaml";

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

/** The block background/foreground Streamdown inherits — the stylesheet owns
 *  the real ones, these just keep a result self-describing. */
const CODE_BG = "#141416";
const CODE_FG = "#e4e4e7";

type Tokenizer = (code: string) => TokenizeResult;

/**
 * Fence language → grammar package key. A name that lands here with no package
 * below (c/cpp/java/…) renders as plain text, exactly like a language no
 * installed grammar ever claimed.
 */
const LANGUAGE_ALIASES: Record<string, string> = {
  ts: "typescript",
  typescript: "typescript",
  js: "javascript",
  javascript: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  jsx: "tsx",
  tsx: "tsx",
  rs: "rust",
  rust: "rust",
  py: "python",
  python: "python",
  pyi: "python",
  go: "go",
  golang: "go",
  sh: "bash",
  bash: "bash",
  zsh: "bash",
  shell: "bash",
  console: "bash",
  shellsession: "shellsession",
  json: "json",
  jsonc: "json",
  toml: "toml",
  yaml: "yaml",
  yml: "yaml",
  md: "markdown",
  markdown: "markdown",
  mdx: "markdown",
  sql: "sql",
  html: "html",
  htm: "html",
  xml: "html",
  svg: "html",
  vue: "html",
  css: "css",
  scss: "css",
  sass: "css",
  svelte: "svelte",
  ini: "ini",
  env: "dotenv",
  dotenv: "dotenv",
  http: "http",
  diff: "diff",
  patch: "diff",
};

/** Grammars the sync callers (tool capsules, raw history) can ask for, built
 *  once at import — the package compiles its tables on import and the factory
 *  only wires the reclassifier pipeline, so neither is per-block work. */
const GRAMMARS: Record<string, LanguageFactory> = {
  bash: bashGrammar,
  css: cssGrammar,
  go: goGrammar,
  html: htmlGrammar,
  javascript: javascriptGrammar,
  json: jsonGrammar,
  markdown: markdownGrammar,
  python: pythonGrammar,
  rust: rustGrammar,
  sql: sqlGrammar,
  toml: tomlGrammar,
  tsx: tsxGrammar,
  typescript: typescriptGrammar,
  yaml: yamlGrammar,
};

/**
 * Grammars fetched on first use rather than at startup: fences no sync caller
 * can request. `diff` is only reachable from a chat fence (tool diffs render
 * through DiffView, not `highlightCodeToHtml`), and the rest are rare fences —
 * either way the block is coloured through the Streamdown plugin's callback, so
 * startup only carries the languages tool output and chat actually use.
 */
const LAZY_GRAMMARS: Record<string, () => Promise<{ tokenize: LanguageFactory }>> = {
  diff: () => import("@twinkleplop/diff"),
  svelte: () => import("@twinkleplop/svelte"),
  ini: () => import("@twinkleplop/ini"),
  http: () => import("@twinkleplop/http"),
  dotenv: () => import("@twinkleplop/dotenv"),
  shellsession: () => import("@twinkleplop/shellsession"),
};

const tokenizers: Record<string, Tokenizer> = {};
for (const [language, factory] of Object.entries(GRAMMARS)) {
  tokenizers[language] = factory();
}

/** In-flight (or settled) lazy imports, so two blocks of the same rare
 *  language share one fetch instead of racing it. */
const lazyImports = new Map<string, Promise<Tokenizer | null>>();

function loadLazy(language: string): Promise<Tokenizer | null> {
  const pending = lazyImports.get(language);
  if (pending) return pending;
  const loader = LAZY_GRAMMARS[language];
  if (!loader) return Promise.resolve(null);
  const promise = loader()
    .then((mod) => {
      const tokenizer = mod.tokenize();
      tokenizers[language] = tokenizer;
      return tokenizer;
    })
    .catch(() => null);
  lazyImports.set(language, promise);
  return promise;
}

/** Fence language → grammar key (`typescript`, `bash`, …). Unknown languages
 *  pass through unchanged and fail the lookup below. */
export function normalizeLanguage(lang?: string): string {
  if (!lang) return "text";
  const clean = lang.toLowerCase().trim();
  return LANGUAGE_ALIASES[clean] || clean;
}

export function supportsLanguage(lang: string): boolean {
  const key = normalizeLanguage(lang);
  return Boolean(tokenizers[key] || LAZY_GRAMMARS[key]);
}

/**
 * Twinkleplop token name → the Prism-style class bucket the stylesheets already
 * colour (light/dark aware, `!important`). Tokens with no bucket keep the
 * theme's colour inline, so nothing is left unpainted — the buckets exist to
 * keep the app's hand-tuned palette, not to gate colour.
 */
const CLASS_BUCKETS: Record<string, string> = {
  comment: "comment",
  doc_marker: "comment",
  directive: "comment",
  keyword: "keyword",
  string: "string",
  template: "string",
  string_escape: "string",
  escape: "string",
  format: "string",
  regex: "regex",
  number: "number",
  hash: "number",
  unit: "number",
  datetime: "number",
  boolean: "boolean",
  constant: "constant",
  null: "constant",
  variant: "constant",
  operator: "operator",
  punctuation: "punctuation",
  attr_sigil: "punctuation",
  list_marker: "punctuation",
  heading_marker: "punctuation",
  blockquote_marker: "punctuation",
  code_fence: "punctuation",
  front_matter_marker: "punctuation",
  function: "function",
  decorator: "function",
  property: "property",
  attr_name: "attr-name",
  attribute: "attr-name",
  class_name: "class-name",
  type: "class-name",
  type_name: "class-name",
  namespace: "class-name",
  builtin: "builtin",
  tag_name: "tag",
  tag: "tag",
  selector: "selector",
  selector_class: "selector",
  selector_id: "selector",
  selector_pseudo: "selector",
  css_variable: "variable",
  variable: "variable",
  parameter: "variable",
  lifetime: "variable",
  expression: "variable",
  doctype: "doctype",
  entity: "entity",
  url: "url",
  url_link: "url",
  url_title: "url",
  autolink: "url",
  link_text: "url",
  inserted: "inserted",
  inserted_marker: "inserted",
  changed: "inserted",
  changed_marker: "inserted",
  deleted: "deleted",
  deleted_marker: "deleted",
  heading: "class-name",
  code: "string",
  code_block: "string",
  code_language: "class-name",
  plain_scalar: "string",
  block_scalar_header: "punctuation",
  array_table_header: "punctuation",
};

/** Whitespace tokens are structural: they split lines and indent, and the
 *  theme never colours them. */
const UNPAINTED = new Set(["space", "tab", "newline", "carriage_return"]);

interface Paint {
  /** Becomes `color` / `--sdm-c` on the rendered span. */
  color?: string;
  className?: string;
}

const EMPTY_PAINT: Paint = {};

/**
 * Colour for a token type from the theme's GitHub Dark palette.
 *
 * Dark only, deliberately: the stylesheet pins every code block to the dark
 * surface (`[data-streamdown="code-block-body"]`, no light variant), so a
 * light-mode palette would paint types the stylesheet has no bucket for nearly
 * invisible on it. `--shiki-dark` carries the same value, which is the hook the
 * stylesheet already reads for dark mode.
 */
function paintOf(name: string | undefined): Paint {
  if (!name || UNPAINTED.has(name)) return EMPTY_PAINT;
  const dark = (themeDark as Record<string, string>)[name];
  const bucket = CLASS_BUCKETS[name];
  return {
    color: dark,
    className: bucket ? `token ${bucket}` : undefined,
  };
}

function styledToken(content: string, paint: Paint): HighlightToken {
  if (!paint.color) return { content, offset: 0 };
  return {
    content,
    color: paint.color,
    htmlStyle: { color: paint.color, "--shiki-dark": paint.color },
    htmlAttrs: paint.className ? { className: paint.className } : undefined,
    offset: 0,
  };
}

/** Append `content` to `lines`, opening a new line per `\n` — a token that
 *  spans lines (block comment, template literal) is split so every line owns
 *  its own tokens. */
function pushContent(lines: HighlightToken[][], content: string, paint: Paint) {
  const parts = content.split("\n");
  for (let i = 0; i < parts.length; i++) {
    if (i > 0) lines.push([]);
    if (parts[i].length > 0) {
      lines[lines.length - 1].push(styledToken(parts[i], paint));
    }
  }
}

/** Flat triplet stream → per-line tokens, keeping the gaps (whitespace the
 *  grammar left as text) and the source order intact. */
function tokensToResult(result: TokenizeResult, code: string): HighlightResult {
  const lines: HighlightToken[][] = [[]];
  const { tokens, token_types } = result;
  let cursor = 0;
  for (let i = 0; i < tokens.length; i += 3) {
    const start = tokens[i + 1];
    const end = tokens[i + 2];
    if (start > cursor) {
      pushContent(lines, code.slice(cursor, start), EMPTY_PAINT);
    }
    pushContent(lines, code.slice(start, end), paintOf(token_types[tokens[i]]));
    cursor = end;
  }
  if (cursor < code.length) {
    pushContent(lines, code.slice(cursor), EMPTY_PAINT);
  }
  return { bg: CODE_BG, fg: CODE_FG, tokens: lines };
}

/** A single unstyled line — the "no grammar" shape both consumers accept. */
function plainLine(content: string): HighlightToken[] {
  return [{ content, offset: 0 }];
}

function plainResult(code: string): HighlightResult {
  return {
    bg: CODE_BG,
    fg: CODE_FG,
    tokens: code.split("\n").map((line) => plainLine(line)),
  };
}

/**
 * Highlight cache — tokenizing from scratch on every re-render would redo work
 * for content that did not change (a sibling row opening, a usage tick), and
 * the streaming tail re-renders on every delta. Caching by content returns the
 * identical object for identical input, so the code block's memo holds.
 *
 * Bounded by entries and total lines, since one entry can be a whole file.
 * Callers treat results as immutable — the streaming plugin copies before
 * appending.
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

function tokenizeWith(language: string, tokenizer: Tokenizer, code: string): HighlightResult {
  const key = cacheKey("tokens", code, language);
  const hit = cacheGet<HighlightResult>(key);
  if (hit) return hit;
  const result = tokensToResult(tokenizer(code), code);
  return cachePut(key, result, result.tokens.length);
}

/**
 * Twinkleplop-backed Streamdown code highlighter.
 */
export const codeHighlightPlugin: StreamdownCodePlugin = {
  name: "shiki",
  type: "code-highlighter",
  getSupportedLanguages() {
    return [...Object.keys(tokenizers), ...Object.keys(LAZY_GRAMMARS)];
  },
  getThemes() {
    return ["github-dark", "github-dark"];
  },
  supportsLanguage(language) {
    return supportsLanguage(language);
  },
  highlight(options, callback) {
    const { code, language } = options;
    const norm = normalizeLanguage(language);
    const tokenizer = tokenizers[norm];
    if (tokenizer) {
      const hit = cacheGet<HighlightResult>(cacheKey("tokens", code, norm));
      if (hit) {
        if (callback) callback(hit);
        return hit;
      }
      if (!callback) return tokenizeWith(norm, tokenizer, code);
      // Cold path — DEFER the tokenize: mount identical-height plain lines
      // now, upgrade colours in idle time. A long session backfilling dozens
      // of code blocks at once used to make that one commit the hotspot; the
      // same line count means zero layout shift, colours just land a beat
      // later.
      const plain = plainResult(code);
      scheduleIdle(() => callback(tokenizeWith(norm, tokenizer, code)));
      return plain;
    }
    if (!LAZY_GRAMMARS[norm]) {
      // No grammar claims this fence — plain lines, same as always.
      const result = plainResult(code);
      cachePut(cacheKey("tokens", code, norm), result, result.tokens.length);
      if (callback) callback(result);
      return result;
    }
    // First use of a rare language: fetch its grammar and colour the block
    // when it lands rather than blocking the commit on a network read.
    const plain = plainResult(code);
    void loadLazy(norm).then((loaded) => {
      if (loaded && callback) callback(tokenizeWith(norm, loaded, code));
    });
    return plain;
  },
};

/**
 * Streaming variant — only for the block that is still arriving (`isAnimating`).
 * Its last line is incomplete, and a half-typed string, comment or regex
 * tokenizes differently on every delta, so that line's colours would flicker and
 * the work is redone for text that is about to change anyway.
 *
 * So the complete-line prefix is highlighted through the shared cache and the
 * trailing partial line renders as plain text; the finished block switches back
 * to this same tokenizer for its exact final colours.
 */
export const codeHighlightPluginStreaming: StreamdownCodePlugin = {
  name: "shiki",
  type: "code-highlighter",
  getSupportedLanguages() {
    return [...Object.keys(tokenizers), ...Object.keys(LAZY_GRAMMARS)];
  },
  getThemes() {
    return ["github-dark", "github-dark"];
  },
  supportsLanguage(language) {
    return supportsLanguage(language);
  },
  highlight(options, callback) {
    const { code, language } = options;
    const cut = code.lastIndexOf("\n");
    let result: HighlightResult;
    if (cut < 0) {
      // Single unterminated line — nothing complete to tokenize yet.
      result = { bg: CODE_BG, fg: CODE_FG, tokens: [plainLine(code)] };
    } else {
      const norm = normalizeLanguage(language);
      const tokenizer = tokenizers[norm];
      const head = code.slice(0, cut + 1);
      const tail = code.slice(cut + 1);
      const headResult = tokenizer ? tokenizeWith(norm, tokenizer, head) : plainResult(head);
      // Copy before appending: `headResult` may be a cache entry, and callers
      // must never see a mutated cache value.
      const tokens = tail ? [...headResult.tokens, plainLine(tail)] : headResult.tokens;
      result = { ...headResult, tokens };
    }
    if (callback) callback(result);
    return result;
  },
};

function scheduleIdle(fn: () => void) {
  if (typeof requestIdleCallback === "function") {
    requestIdleCallback(() => fn());
    return;
  }
  setTimeout(fn, 0);
}

/**
 * Highlighted HTML string for direct rendering (tool capsules, raw history).
 * Token colour comes from the same palette: the classed buckets are coloured by
 * the stylesheet (light/dark aware), and a token type with no bucket keeps the
 * container's colour rather than forcing a mode-blind inline colour.
 */
export function highlightCodeToHtml(code: string, language: string): string {
  const norm = normalizeLanguage(language);
  const tokenizer = tokenizers[norm];
  if (!tokenizer || !code) return escapeHtml(code);
  const key = cacheKey("html", code, norm);
  const hit = cacheGet<string>(key);
  if (hit !== undefined) return hit;
  const { tokens } = tokenizeWith(norm, tokenizer, code);
  const html = tokens
    .map((line) =>
      line
        .map((token) =>
          token.htmlAttrs?.className
            ? `<span class="${token.htmlAttrs.className}">${escapeHtml(token.content)}</span>`
            : escapeHtml(token.content)
        )
        .join("")
    )
    .join("\n");
  return cachePut(key, html, countLines(code));
}

function escapeHtml(str: string): string {
  return str
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}
