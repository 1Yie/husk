// Simulate the desktop boundary, not xterm. Real DOM/canvas/font layout and
// the actual terminal/session code run in the browser.
export class Channel { onmessage = (_event: ArrayBuffer) => {}; }
type Args = { cwd?: string; id?: string; path?: string; cols?: number; rows?: number; output?: Channel; op?: string };
type Metrics = { start: number; ipc: unknown[] };
const resizing = new URLSearchParams(location.search).has("resize");
let nextId = 0;
const outputs = new Map<string, Channel>();
function redraw(output: Channel | undefined, cols = 80, rows = 24) {
  const lines = Array.from({ length: Math.max(1, rows - 1) }, (_, i) =>
    `\x1b[3${i % 7 + 1}m${`row ${i} | terminal output `.padEnd(Math.max(1, cols - 2), ".")}\x1b[0m`,
  ).join("\r\n");
  output?.onmessage(new TextEncoder().encode(`\x1b[H${lines}`).buffer);
}
export async function invoke(command: string, args: Args = {}) {
  const metrics = (window as unknown as { __terminalMetrics?: Metrics }).__terminalMetrics;
  if (command.startsWith("terminal_")) {
    metrics?.ipc.push({ command, at: performance.now() - metrics.start, ...args, output: undefined });
  }
  if (command === "terminal_open") {
    await new Promise((resolve) => setTimeout(resolve, 80));
    const id = `pty-fixture-${++nextId}`;
    const output = args.output;
    if (output) outputs.set(id, output);
    if (resizing) {
      const data = new TextEncoder().encode(Array.from({ length: 3000 }, (_, i) =>
        `history ${i} ${"scrollback content ".repeat(8)}\r\n`,
      ).join(""));
      for (let i = 0; i < data.length; i += 8192) output?.onmessage(data.slice(i, i + 8192).buffer);
      output?.onmessage(new TextEncoder().encode("\x1b[?1049h").buffer);
      redraw(output, args.cols, args.rows);
    }
    args.output?.onmessage(new TextEncoder().encode("\x1b[32mhusk\x1b[0m on main\r\n❯ ").buffer);
    return { id, cwd: args.cwd ?? "/repo", shell: "/bin/sh" };
  }
  if (command === "terminal_resize" && resizing) {
    // A slow native round-trip plus a TUI's SIGWINCH redraw. Instant resize
    // mocks and an empty terminal concealed the obsolete-size backlog.
    await new Promise((resolve) => setTimeout(resolve, 200));
    redraw(args.id ? outputs.get(args.id) : undefined, args.cols, args.rows);
  }
  if (command === "terminal_close" && args.id) outputs.delete(args.id);
  if (command === "workspace_list_files") return { path: args.path ?? "", entries: [], truncated: false };
  if (command === "agent_session") {
    if (args.op === "model_info") return { active_provider: "fixture", active_model: "fixture", models: [] };
    if (args.op === "git_info") return { repo: true, branch: "main", dirty: 0 };
    if (args.op === "workspace_changes") return { repo: false, files: [] };
    if (args.op === "linked_roots") return { workspace: [], session: [] };
    if (args.op?.startsWith("list_") || args.op === "bg_list" || args.op === "plugin_commands") return [];
  }
  return {};
}
export const convertFileSrc = (src: string) => src;
export const listen = async () => () => {};
export const emit = async () => {};
export const getCurrentWindow = () => ({
  onFocusChanged: async () => () => {}, isFocused: async () => true,
  startDragging: async () => {}, toggleMaximize: async () => {},
  isMaximized: async () => false, onResized: async () => () => {},
  minimize: async () => {}, close: async () => {},
});
