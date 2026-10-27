import type { CSSProperties } from "react";
import themeData from "@/assets/vscode-seti/vs-seti-icon-theme.json";
import "./file-icon.css";

interface Associations {
  file?: string;
  fileNames?: Record<string, string>;
  fileExtensions?: Record<string, string>;
  languageIds?: Record<string, string>;
}
interface SetiTheme extends Associations {
  iconDefinitions: Record<string, { fontCharacter: string; fontColor?: string }>;
  light: Associations;
}
const theme: SetiTheme = themeData;
const light: Associations = {
  file: theme.light.file ?? theme.file,
  fileNames: { ...theme.fileNames, ...theme.light.fileNames },
  fileExtensions: { ...theme.fileExtensions, ...theme.light.fileExtensions },
  languageIds: { ...theme.languageIds, ...theme.light.languageIds },
};
const languageByExtension = new Map(Object.entries({
  js: "javascript", mjs: "javascript", cjs: "javascript",
  jsx: "javascriptreact", ts: "typescript", mts: "typescript", cts: "typescript",
  tsx: "typescriptreact", json: "json", jsonc: "jsonc", jsonl: "jsonl",
  md: "markdown", markdown: "markdown", py: "python", pyw: "python",
  rs: "rust", go: "go", rb: "ruby", html: "html", htm: "html", css: "css",
  scss: "scss", sass: "sass", less: "less", vue: "vue", svelte: "svelte",
  sh: "shellscript", bash: "shellscript", zsh: "shellscript", fish: "shellscript",
  ps1: "powershell", bat: "bat", cmd: "bat", yml: "yaml", yaml: "yaml",
  c: "c", h: "c", cc: "cpp", cpp: "cpp", hpp: "cpp", cs: "csharp",
  java: "java", kt: "kotlin", swift: "swift", sql: "sql", xml: "xml",
  php: "php", lua: "lua", dart: "dart",
}));

function index(associations: Associations) {
  const entries = (values?: Record<string, string>) =>
    new Map(Object.entries(values ?? {}).map(([name, icon]) => [name.toLowerCase(), icon]));
  return {
    names: entries(associations.fileNames),
    extensions: entries(associations.fileExtensions),
    languages: entries(associations.languageIds),
    fallback: associations.file ?? "_default",
  };
}
const darkIndex = index(theme);
const lightIndex = index(light);

function resolveIcon(name: string, associations: ReturnType<typeof index>) {
  const filename = (name.split(/[\\/]/).pop() ?? name).toLowerCase();
  let id = associations.names.get(filename);
  const parts = filename.split(".");
  // Compound associations such as declarations/config files take precedence.
  for (let start = 1; !id && start < parts.length; start++) {
    id = associations.extensions.get(parts.slice(start).join("."));
  }
  const language = filename.startsWith(".env") ? "dotenv"
    : languageByExtension.get(parts[parts.length - 1]);
  id ??= language ? associations.languages.get(language) : undefined;
  id ??= associations.fallback;
  const definition = theme.iconDefinitions[id] ?? theme.iconDefinitions._default;
  return {
    name: id,
    glyph: String.fromCodePoint(Number.parseInt(definition.fontCharacter.slice(1), 16)),
    color: id.startsWith("_default") ? "var(--husk-fg-muted)" : definition.fontColor ?? "var(--husk-fg-muted)",
  };
}

export function workspaceFileIcon(name: string) {
  return { light: resolveIcon(name, lightIndex), dark: resolveIcon(name, darkIndex) };
}

export function WorkspaceFileIcon({ name, className = "" }: { name: string; className?: string }) {
  const icon = workspaceFileIcon(name);
  return (
    <span className={`workspace-file-icon ${className}`} aria-hidden="true"
      data-file-icon={icon.dark.name.slice(1)}
      style={{ "--file-icon-light": icon.light.color, "--file-icon-dark": icon.dark.color } as CSSProperties}>
      <span className="workspace-file-icon__light">{icon.light.glyph}</span>
      <span className="workspace-file-icon__dark">{icon.dark.glyph}</span>
    </span>
  );
}
