// The builtin slash-command catalog — the ONE list behind the composer's `/`
// panel and the chat stream's `/command` chip. `send` entries are dispatched by
// the kernel as typed text; `local` entries drive the desktop UI and must never
// be sent, since the kernel would only answer them with a text placeholder.

export type CommandKind = "send" | "local";

export interface BuiltinCommand {
  name: string;
  desc: string;
  kind: CommandKind;
  /** Argument placeholder, shown after the name in the panel. The pick inserts
   *  `/name ` and leaves the caret there for the value. */
  args?: string;
}

export const BUILTIN_COMMANDS: readonly BuiltinCommand[] = [
  { name: "clear", desc: "清空当前会话", kind: "send" },
  { name: "compact", desc: "压缩上下文", kind: "send" },
  { name: "undo", desc: "撤销上一轮的文件修改", kind: "send" },
  { name: "remember", desc: "记住一条偏好", kind: "send", args: "<内容>" },
  { name: "blueprint", desc: "架构蓝图", kind: "send", args: "[init|check|graph]" },
  // `forget` is deliberately absent: it still works typed, but forgetting a
  // preference is not a thing to advertise in a desktop picker.
  { name: "model", desc: "切换模型", kind: "local" },
  { name: "diff", desc: "查看改动", kind: "local" },
  { name: "skills", desc: "管理技能", kind: "local" },
  { name: "memory", desc: "管理记忆", kind: "local" },
];

/** The entry for a bare command name (no leading slash). */
export function findCommand(name: string): BuiltinCommand | undefined {
  return BUILTIN_COMMANDS.find((cmd) => cmd.name === name);
}

/** A sent command's description — what the stream's chip states in its tooltip. */
export function commandDesc(name: string): string | undefined {
  return findCommand(name)?.desc;
}

/** The local command a bare submission names, if any — Enter on `/model` must
 *  run the picker instead of sending the word to the kernel. Anything with
 *  arguments stays text: `/diff HEAD~1` is the user's business, not ours. */
export function localCommandOf(text: string): BuiltinCommand | undefined {
  const match = /^\/(\S+)$/.exec(text.trim());
  if (!match) return undefined;
  const cmd = findCommand(match[1]);
  return cmd?.kind === "local" ? cmd : undefined;
}
