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
 * Tokenize pure code into HighlightResult (lines of tokens) compatible with Streamdown
 */
export function highlightCodeWithPrism(code: string, language: string): HighlightResult {
  const grammar = getPrismGrammar(language);
  if (!grammar) {
    const lines = code.split("\n");
    return {
      bg: "#141416",
      fg: "#e4e4e7",
      tokens: lines.map((line) => [
        {
          content: line || "",
          color: "#e4e4e7",
          bgColor: "transparent",
          htmlStyle: {
            color: "#e4e4e7",
            "--sdm-c": "#e4e4e7",
            "--shiki-dark": "#e4e4e7",
          },
          offset: 0,
        },
      ]),
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
      lines[i].push({
        content: "",
        color: "#e4e4e7",
        bgColor: "transparent",
        htmlStyle: {
          color: "#e4e4e7",
          "--sdm-c": "#e4e4e7",
          "--shiki-dark": "#e4e4e7",
        },
        offset: 0,
      });
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
  const grammar = getPrismGrammar(language);
  if (!grammar) {
    return escapeHtml(code);
  }
  const norm = normalizeLanguage(language);
  try {
    return Prism.highlight(code, grammar, norm);
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
