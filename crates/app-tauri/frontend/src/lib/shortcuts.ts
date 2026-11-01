/** Application shortcuts: the single definition of what a binding means.
 *
 * Bindings are stored as strings such as `mod+shift+j`, so the same value can
 * be persisted, compared against a keyboard event and rendered through
 * `shortcutLabel` without a second spelling per platform.
 */
import { isMac, shortcutLabel } from "@/lib/platform";

export type ShortcutAction =
  | "terminal"
  | "session-sidebar"
  | "tools-sidebar"
  | "settings"
  | "new-session"
  | "workspace-file"
  | "mode-build"
  | "mode-office";

/** `toggle` bindings only open or close chrome; `workspace` bindings create
 *  sessions or move the workspace, so a focused text field keeps its keys. */
export type ShortcutScope = "toggle" | "workspace";

export interface ShortcutDefinition {
  action: ShortcutAction;
  label: string;
  /** One line saying what the action does, like every other settings row. */
  description: string;
  group: string;
  scope: ShortcutScope;
  /** macOS and other platforms differ only in the glyphs `mod` renders as. */
  defaultBinding: string;
}

const mod = "mod";

export const SHORTCUTS: ShortcutDefinition[] = [
  { action: "terminal", label: "打开／收起终端", description: "在窗口底部打开本机 shell", group: "面板", scope: "toggle", defaultBinding: `${mod}+shift+j` },
  { action: "session-sidebar", label: "显示／隐藏会话侧栏", description: "左侧的项目与会话列表", group: "面板", scope: "toggle", defaultBinding: `${mod}+shift+l` },
  { action: "tools-sidebar", label: "显示／隐藏工具侧栏", description: "右侧的改动、产物与后台任务", group: "面板", scope: "toggle", defaultBinding: `${mod}+shift+b` },
  { action: "settings", label: "打开设置", description: "打开设置中的常规页", group: "面板", scope: "toggle", defaultBinding: `${mod}+,` },
  { action: "new-session", label: "新建会话", description: "在当前工作区开始下一次会话", group: "工作区", scope: "workspace", defaultBinding: `${mod}+n` },
  { action: "workspace-file", label: "打开工作区文件", description: "在右侧面板浏览工作区文件", group: "工作区", scope: "workspace", defaultBinding: `${mod}+shift+e` },
  { action: "mode-build", label: "切换到编程模式", description: "回到最近使用的项目", group: "工作区", scope: "workspace", defaultBinding: `${mod}+1` },
  { action: "mode-office", label: "切换到工作模式", description: "打开内置的 Word / Excel / PPT 工作区", group: "工作区", scope: "workspace", defaultBinding: `${mod}+2` },
];

export const SHORTCUT_ACTIONS = SHORTCUTS.map((item) => item.action);

export function shortcutDefinition(action: ShortcutAction): ShortcutDefinition {
  const found = SHORTCUTS.find((item) => item.action === action);
  if (!found) throw new Error(`unknown shortcut action: ${action}`);
  return found;
}

export function defaultBindings(): Record<ShortcutAction, string> {
  const bindings = {} as Record<ShortcutAction, string>;
  for (const item of SHORTCUTS) bindings[item.action] = item.defaultBinding;
  return bindings;
}

/** Modifier-only keydowns describe a held modifier, not a binding. */
const MODIFIER_KEYS = new Set(["Shift", "Control", "Alt", "Meta", "CapsLock", "Fn", "Dead"]);

function normalizedKey(key: string): string {
  return key.length === 1 ? key.toLowerCase() : key.toLowerCase();
}

/** The binding a keyboard event expresses, or null when it is only modifiers. */
export function bindingFromEvent(
  event: Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "shiftKey" | "altKey">,
): string | null {
  if (!event.key || MODIFIER_KEYS.has(event.key)) return null;
  const parts: string[] = [];
  if (isMac ? event.metaKey : event.ctrlKey) parts.push("mod");
  // The non-mod primary key is still shown so Mac users can reach Ctrl-based
  // combinations that macOS itself does not bind for us.
  else if (isMac && event.ctrlKey) parts.push("ctrl");
  else if (!isMac && event.metaKey) parts.push("meta");
  if (event.altKey) parts.push("alt");
  if (event.shiftKey) parts.push("shift");
  const key = normalizedKey(event.key);
  if (!key) return null;
  parts.push(key);
  return parts.join("+");
}

/** Glyphs for the modifiers `shortcutLabel` does not own. */
const EXTRA_MODIFIERS: Record<string, { mac: string; other: string }> = {
  ctrl: { mac: "^", other: "Ctrl" },
  meta: { mac: "⌘", other: "Win" },
};

/** Render a stored binding the way this platform writes shortcuts. */
export function bindingLabel(binding: string | null | undefined, mac = isMac): string {
  if (!binding) return "";
  const parts = binding.split("+");
  const key = parts.pop() ?? "";
  const modifiers = parts.map((part) => {
    const extra = EXTRA_MODIFIERS[part];
    if (!extra) return part;
    return mac ? extra.mac : extra.other;
  });
  return shortcutLabel([...modifiers, key], mac);
}

/** The action this event triggers, ignoring repeat and composition. */
export function actionForEvent(
  event: KeyboardEvent,
  bindings: Partial<Record<ShortcutAction, string>>,
): ShortcutAction | null {
  if (event.repeat || event.isComposing) return null;
  const binding = bindingFromEvent(event);
  if (!binding) return null;
  for (const action of SHORTCUT_ACTIONS) {
    if (bindings[action] === binding) return action;
  }
  return null;
}

/** Report which action already owns a binding, so the editor can refuse it. */
export function bindingOwner(
  binding: string,
  bindings: Partial<Record<ShortcutAction, string>>,
  except?: ShortcutAction,
): ShortcutAction | null {
  for (const action of SHORTCUT_ACTIONS) {
    if (action !== except && bindings[action] === binding) return action;
  }
  return null;
}
