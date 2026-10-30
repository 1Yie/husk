import { Channel, invoke } from "@tauri-apps/api/core";
import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { isMac } from "@/lib/platform";
import type { TerminalCursorStyle } from "./preferences";
import { isLayoutResizing } from "@/lib/layout-resize";
import { queueTerminalWork } from "./render-queue";

/** Output arrives as raw bytes (an `ArrayBuffer`); exit and error as JSON. */
export type TerminalEvent =
  | ArrayBuffer
  | { type: "exit"; code: number | null; signal: string | null }
  | { type: "error"; message: string };

type OpenResult = { id: string; cwd: string; shell: string };
export type SessionState = {
  status: "opening" | "running" | "exited" | "error";
  cwd?: string;
  shell?: string;
  /** The title the shell set with OSC 0/2, if any. */
  title?: string;
  code?: number | null;
  signal?: string | null;
  message?: string;
};

/** Everything about the renderer that 设置 › 终端 and the app theme decide. */
export interface TerminalAppearance {
  theme: ITheme;
  fontFamily: string;
  fontSize: number;
  lineHeight: number;
  cursorStyle: TerminalCursorStyle;
  cursorBlink: boolean;
  scrollback: number;
}

const validSize = (cols: number, rows: number) =>
  Number.isInteger(cols) && Number.isInteger(rows) && cols > 0 && rows > 0;
const MAX_STARTUP_REPLY_CHARS = 64 * 1024;

function bytesOf(message: unknown): Uint8Array | null {
  if (message instanceof ArrayBuffer) return new Uint8Array(message);
  if (ArrayBuffer.isView(message))
    return new Uint8Array(
      message.buffer,
      message.byteOffset,
      message.byteLength,
    );
  if (Array.isArray(message)) return new Uint8Array(message);
  return null;
}

/** A key the clipboard owns, never the shell. (Ctrl/\u2318 Shift J never gets
 *  here: MainLayout takes it in the capture phase.) */
function shortcutOf(event: KeyboardEvent): "copy" | "paste" | null {
  if (event.type !== "keydown" || event.altKey) return null;
  const key = event.key.toLowerCase();
  // macOS: ⌘C / ⌘V never reach a shell anyway. Elsewhere Ctrl+C is SIGINT,
  // so copy and paste take the Shift layer, as in every Linux terminal.
  const mod = isMac
    ? event.metaKey && !event.ctrlKey
    : event.ctrlKey && event.shiftKey && !event.metaKey;
  if (!mod) return null;
  if (key === "c") return "copy";
  if (key === "v") return "paste";
  return null;
}

/** Owns exactly one PTY and renderer. Hiding never calls dispose(). */
export class TerminalSession {
  private readonly terminal: Terminal;
  private readonly fitAddon = new FitAddon();
  private readonly output = new Channel<TerminalEvent>();
  private readonly subscriptions: { dispose(): void }[] = [];
  private id: string | undefined;
  private disposed = false;
  private visible = false;
  private focusPending = false;
  private fitTask: (() => void) | undefined;
  private fitTimer: number | undefined;
  private immediateFit = false;
  private writes = Promise.resolve();
  private startupReplies = "";
  private colorReplyRestoreQueued = false;
  private resizing = false;
  private pendingSize: { cols: number; rows: number } | undefined;
  private lastSize = "";
  private appearanceKey = "";
  private appearance: TerminalAppearance;
  private fontEpoch = 0;
  private fontFamily = "";
  private fittedGeometry: { width: number; height: number; dpr: number; fontEpoch: number } | undefined;
  private state: SessionState;

  constructor(
    private readonly host: HTMLElement,
    cwd: string | undefined,
    private readonly onState: (state: SessionState) => void,
    appearance: TerminalAppearance,
    private readonly readClipboard: () => Promise<string | null> = async () =>
      null,
    private readonly canAutoFocus: () => boolean = () => true,
  ) {
    this.state = { status: "opening", cwd };
    this.appearance = appearance;
    this.appearanceKey = JSON.stringify(appearance);
    this.fontFamily = appearance.fontFamily;
    this.terminal = new Terminal({
      // Give xterm its final font/theme before opening. Applying these after
      // open rebuilt its measurements/renderer repeatedly on the first click.
      ...appearance,
      // fitNow installs the actual dimensions before spawning. Do not build
      // a throwaway 80×24 screen and immediately resize it again.
      cols: 1,
      rows: 1,
      disableStdin: true,
      allowProposedApi: false,
      // No clipboard addon/OSC 52 handler, web-links addon, or URL activation.
      linkHandler: { activate: () => {} },
    });
    try {
      this.terminal.loadAddon(this.fitAddon);
      this.terminal.open(host);
      this.terminal.attachCustomKeyEventHandler((event) =>
        this.handleKey(event),
      );
      this.subscriptions.push(
        this.terminal.onData((data) => this.write(data)),
        this.terminal.onTitleChange((title) => {
          const value = title.trim() || undefined;
          if (!this.disposed && value !== this.state.title)
            this.update({ ...this.state, title: value });
        }),
      );
      for (const id of [4, 10, 11, 12]) {
        this.subscriptions.push(this.terminal.parser.registerOscHandler(id, (data) =>
          this.allowColorReply(data)));
      }
    } catch (error) {
      this.terminal.dispose();
      throw error;
    }
    this.output.onmessage = (event) => this.receive(event);
    // Spawn at the size the screen will show: a shell started at 80×24 and
    // resized a frame later draws its first prompt for the wrong width.
    if (!this.fitNow()) this.terminal.resize(80, 24);
    const openingSize = { cols: this.terminal.cols, rows: this.terminal.rows };
    // Install the channel callback before spawning; output may precede the reply.
    void invoke<OpenResult>("terminal_open", {
      ...(cwd === undefined ? {} : { cwd }),
      ...openingSize,
      output: this.output,
    })
      .then((result) => {
        if (this.disposed) {
          this.closeBackend(result.id);
          return;
        }
        this.id = result.id;
        // The renderer may have fitted again while spawn was pending. The
        // backend still has the requested size, not that newer renderer size.
        this.lastSize = `${openingSize.cols}:${openingSize.rows}`;
        this.update({
          ...this.state,
          cwd: result.cwd,
          shell: result.shell,
          status:
            this.state.status === "opening" ? "running" : this.state.status,
        });
        const replies = this.startupReplies;
        this.startupReplies = "";
        if (replies) this.write(replies);
        this.scheduleFit({ immediate: true });
      })
      .catch(() => {
        if (!this.disposed) {
          this.update({
            ...this.state,
            status: "error",
            message: "无法启动终端，请重试。",
          });
        }
      });
  }

  private receive(event: TerminalEvent) {
    if (this.disposed) return;
    const bytes = bytesOf(event);
    if (bytes) {
      // xterm's streaming UTF-8 decoder retains partial multibyte sequences.
      this.terminal.write(bytes);
    } else if (!("type" in event)) {
      return;
    } else if (event.type === "exit") {
      this.update({
        ...this.state,
        status: "exited",
        code: event.code,
        signal: event.signal,
      });
    } else if (this.state.status !== "exited") {
      this.update({
        ...this.state,
        status: "error",
        message: "终端连接出错，请重新启动。",
      });
    }
  }

  private update(state: SessionState) {
    this.state = state;
    if (state.status === "exited" || state.status === "error") this.startupReplies = "";
    this.syncInput();
    this.onState(state);
  }

  private syncInput() {
    this.terminal.options.disableStdin =
      this.disposed || !this.visible || this.state.status !== "running";
  }

  private allowColorReply(data: string): false {
    if (this.disposed || (this.state.status !== "opening" && this.state.status !== "running")
      || !data.split(";").includes("?")) return false;
    // disableStdin also suppresses xterm's own OSC replies. Allow its native
    // color handler to answer in this parser turn, then restore the current
    // input gate before another browser input event can run.
    if (this.terminal.options.disableStdin) {
      this.terminal.options.disableStdin = false;
      if (!this.colorReplyRestoreQueued) {
        this.colorReplyRestoreQueued = true;
        queueMicrotask(() => {
          this.colorReplyRestoreQueued = false;
          if (!this.disposed) this.syncInput();
        });
      }
    }
    // Never consume or reinterpret the query: xterm owns palette/RGB values.
    return false;
  }

  private handleKey(event: KeyboardEvent): boolean {
    const shortcut = shortcutOf(event);
    if (!shortcut) return true;
    if (shortcut === "copy") {
      // Nothing selected: ⌘C on macOS stays a no-op rather than a ^C.
      if (!this.terminal.hasSelection()) return isMac ? false : true;
      event.preventDefault();
      this.copySelection();
      return false;
    }
    event.preventDefault();
    void this.pasteFromClipboard();
    return false;
  }

  private write(data: string) {
    if (this.disposed) return;
    if (!this.id && this.state.status === "opening") {
      // Keyboard input is disabled here; parser-generated replies may arrive
      // before terminal_open acknowledges the PTY id.
      if (this.startupReplies.length + data.length <= MAX_STARTUP_REPLY_CHARS)
        this.startupReplies += data;
      return;
    }
    if (this.state.status !== "running" || !this.id) return;
    // Serialize chunks across onData events; never split a UTF-16 surrogate pair.
    for (let start = 0; start < data.length; ) {
      let end = Math.min(start + 4096, data.length);
      const last = data.charCodeAt(end - 1);
      if (end < data.length && last >= 0xd800 && last <= 0xdbff) end--;
      const chunk = data.slice(start, end);
      start = end;
      this.writes = this.writes
        .then(async () => {
          if (this.disposed || this.state.status !== "running" || !this.id)
            return;
          await invoke("terminal_write", { id: this.id, data: chunk });
        })
        .catch(() => {
          if (!this.disposed && this.state.status === "running") {
            this.update({
              ...this.state,
              status: "error",
              message: "终端输入失败，请重新启动。",
            });
          }
        });
    }
  }

  setVisible(visible: boolean, focus = true) {
    this.visible = visible;
    this.syncInput();
    if (visible) {
      this.focusPending = focus;
      if (!this.fittedGeometry) this.remeasureFonts();
      this.scheduleFit({ immediate: true });
    } else {
      this.focusPending = false;
      this.pendingSize = undefined;
      this.terminal.blur();
      this.cancelFit();
    }
  }

  /** Fit to the host if it has a size; false when there was nothing to fit. */
  private fitNow(): boolean {
    const width = this.host.clientWidth;
    const height = this.host.clientHeight;
    if (
      this.disposed ||
      !this.host.isConnected ||
      width <= 0 ||
      height <= 0
    )
      return false;
    const previous = this.fittedGeometry;
    const dpr = window.devicePixelRatio;
    if (previous && previous.width === width && previous.height === height
      && previous.dpr === dpr && previous.fontEpoch === this.fontEpoch) return true;
    const size = this.fitAddon.proposeDimensions();
    if (!size || !validSize(size.cols, size.rows)) return false;
    this.fitAddon.fit();
    this.fittedGeometry = { width, height, dpr, fontEpoch: this.fontEpoch };
    return true;
  }

  /** ResizeObserver only schedules work, never mutates layout synchronously. */
  scheduleFit({ immediate = false, widthChanged = false }: { immediate?: boolean; widthChanged?: boolean } = {}) {
    if (this.disposed || !this.visible || isLayoutResizing()) return;
    if (immediate) {
      this.immediateFit = true;
      if (this.fitTimer !== undefined) window.clearTimeout(this.fitTimer);
      this.fitTimer = undefined;
    } else if (widthChanged && !this.immediateFit) {
      // Changing columns reflows the entire scrollback, unlike a height
      // change. Do not repeat that work for every pixel of a window drag.
      this.fitTask?.();
      this.fitTask = undefined;
      if (this.fitTimer !== undefined) window.clearTimeout(this.fitTimer);
      this.fitTimer = window.setTimeout(() => {
        this.fitTimer = undefined;
        this.scheduleFit();
      }, 64);
      return;
    } else if (this.fitTimer !== undefined) {
      return;
    }
    if (this.fitTask !== undefined) return;
    this.fitTask = queueTerminalWork(() => {
      this.fitTask = undefined;
      this.immediateFit = false;
      if (!this.visible || isLayoutResizing() || !this.fitNow()) return;
      const { cols, rows } = this.terminal;
      if (
        this.id &&
        this.state.status === "running" &&
        validSize(cols, rows)
      ) {
        this.resizeTo(cols, rows);
      }
      if (this.focusPending) {
        this.focusPending = false;
        if (this.canAutoFocus()) this.terminal.focus();
      }
    }, () => this.focusPending);
  }

  private cancelFit() {
    this.fitTask?.();
    if (this.fitTimer !== undefined) window.clearTimeout(this.fitTimer);
    this.fitTask = undefined;
    this.fitTimer = undefined;
    this.immediateFit = false;
  }

  private resizeTo(cols: number, rows: number) {
    if (!this.resizing && this.lastSize === `${cols}:${rows}`) return;
    // A slow IPC round-trip must not serialize every intermediate drag
    // position. Keep one in flight and overwrite just the pending target.
    this.pendingSize = { cols, rows };
    if (this.resizing) return;
    this.resizing = true;
    void this.sendLatestSize();
  }

  private async sendLatestSize() {
    try {
      while (!this.disposed && this.visible && this.id && this.state.status === "running") {
        const size = this.pendingSize;
        this.pendingSize = undefined;
        if (!size) break;
        const key = `${size.cols}:${size.rows}`;
        if (key === this.lastSize) continue;
        await invoke("terminal_resize", { id: this.id, ...size });
        this.lastSize = key;
      }
    } catch {
      this.lastSize = "";
      if (!this.disposed && this.state.status === "running") {
        this.update({ ...this.state, status: "error", message: "终端尺寸同步失败，请重新启动。" });
      }
    } finally {
      this.resizing = false;
      this.pendingSize = undefined;
    }
  }

  setAppearance(appearance: TerminalAppearance) {
    if (this.disposed) return;
    const key = JSON.stringify(appearance);
    if (key === this.appearanceKey) return;
    this.appearanceKey = key;
    const previous = this.appearance;
    this.appearance = appearance;
    const options = this.terminal.options;
    // xterm option setters can invalidate renderer state even when assigned
    // an equivalent value. A palette click must not reset font measurements.
    if (JSON.stringify(previous.theme) !== JSON.stringify(appearance.theme))
      options.theme = appearance.theme;
    if (previous.fontSize !== appearance.fontSize) options.fontSize = appearance.fontSize;
    if (previous.lineHeight !== appearance.lineHeight) options.lineHeight = appearance.lineHeight;
    if (previous.cursorStyle !== appearance.cursorStyle) options.cursorStyle = appearance.cursorStyle;
    if (previous.cursorBlink !== appearance.cursorBlink) options.cursorBlink = appearance.cursorBlink;
    if (previous.scrollback !== appearance.scrollback) options.scrollback = appearance.scrollback;
    if (previous.fontFamily !== appearance.fontFamily) {
      this.fontFamily = appearance.fontFamily;
      options.fontFamily = this.fontFamily;
    }
    if (previous.fontSize !== appearance.fontSize || previous.lineHeight !== appearance.lineHeight
      || previous.fontFamily !== appearance.fontFamily) {
      this.fittedGeometry = undefined;
      this.scheduleFit({ immediate: true });
    }
  }

  /** A web font finished loading: xterm measured its cells against the
   *  fallback and only re-measures when the family string changes, so the
   *  string alternates between two spellings of the same stack. */
  remeasureFonts() {
    if (this.disposed) return;
    this.fontEpoch++;
    this.terminal.options.fontFamily =
      this.fontEpoch % 2 ? `${this.fontFamily}, monospace` : this.fontFamily;
    this.scheduleFit({ immediate: true });
  }

  hasSelection() {
    return !this.disposed && this.terminal.hasSelection();
  }

  copySelection() {
    const text = this.terminal.getSelection();
    if (text) void navigator.clipboard?.writeText(text).catch(() => {});
  }

  async pasteFromClipboard() {
    const document = this.host.ownerDocument;
    const owner = document.activeElement;
    let text: string | null = null;
    try {
      text = await this.readClipboard();
    } catch {
      text = null;
    }
    // `paste` applies bracketed-paste mode when the shell asked for it, so a
    // multi-line paste is not run line by line.
    if (text && !this.disposed && this.state.status === "running")
      this.terminal.paste(text);
    const active = document.activeElement;
    // A portalled menu can disappear while reading, leaving body focused.
    // Restore that explicit paste target, but never override a new focus owner.
    if (active === owner || this.host.contains(active)
      || (active === document.body && owner && !owner.isConnected))
      this.focus();
  }

  selectAll() {
    if (!this.disposed) this.terminal.selectAll();
  }

  /** Clears the scrollback and screen, keeping the prompt line. */
  clear() {
    if (!this.disposed) this.terminal.clear();
  }

  focus() {
    if (!this.disposed && this.visible) this.terminal.focus();
  }

  private closeBackend(id: string) {
    // Backend close is idempotent. Cleanup must not create unhandled rejections.
    void invoke("terminal_close", { id }).catch(() => {});
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.startupReplies = "";
    this.pendingSize = undefined;
    this.output.onmessage = () => {};
    this.cancelFit();
    for (const subscription of this.subscriptions) subscription.dispose();
    this.terminal.dispose();
    if (this.id) this.closeBackend(this.id);
  }
}
