export class Channel { onmessage = (_event: unknown) => {}; }

type Output = { onmessage: (event: unknown) => void };
export const shell = {
  opened: false,
  opens: 0,
  closes: 0,
  resizes: 0,
  queriedBeforeOpen: false,
  writes: [] as string[],
  promptSent: false,
};
let output: Output | undefined;
const send = (data: string) => output?.onmessage(new TextEncoder().encode(data).buffer);
const queries = "\x1b]11;?\x07\x1b]10;?\x1b\\\x1b]4;1;?\x1b\\\x1b]12;?\x1b\\";

export function probeHidden() { send(queries.replaceAll("\x1b\\", "\x07")); }
export function setShellTheme() {
  send("\x1b]10;#d5e6f7\x1b\\\x1b]11;#24364a\x1b\\\r\nSHELLDEFAULT\r\n");
}

export async function invoke(command: string, args?: Record<string, unknown>) {
  if (command === "terminal_open") {
    shell.opens++;
    output = args!.output as Output;
    setTimeout(() => {
      shell.queriedBeforeOpen = !shell.opened;
      send("\x1b]4;1;#9f3a6d\x1b\\" + queries);
    }, 0);
    // A real PTY can produce a prompt before its invoke reply reaches the
    // webview. Hold that reply long enough to exercise the ordering reliably.
    await new Promise((resolve) => setTimeout(resolve, 600));
    shell.opened = true;
    return { id: "colors-pty", cwd: "/fixture", shell: "/bin/bash" };
  }
  if (command === "terminal_write") {
    const data = String(args!.data);
    shell.writes.push(data);
    if (!shell.promptSent && data.includes("]11;rgb:")) {
      shell.promptSent = true;
      send(
        "\x1b[31mANSI16\x1b[0m\r\n" +
        "\x1b[38;5;135mANSI256\x1b[0m\r\n" +
        "\x1b[38;2;37;149;233mTRUECOLOR\x1b[0m\r\n" +
        "\x1b[48;2;20;60;100mBACKGROUND\x1b[0m\r\n" +
        "\x1b[38;2;204;102;221m❯ \x1b[38;2;91;190;102muser@host " +
        "\x1b[38;2;60;140;230m~/workspace\x1b[0m ",
      );
    }
    return;
  }
  if (command === "terminal_resize") { shell.resizes++; return; }
  if (command === "terminal_close") { shell.closes++; return; }
  return {};
}

export const listen = async () => () => {};
export const emit = async () => {};
export const convertFileSrc = (path: string) => path;
