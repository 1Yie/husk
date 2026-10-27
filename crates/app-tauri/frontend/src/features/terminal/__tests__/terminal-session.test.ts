import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { TerminalSession, type SessionState, type TerminalAppearance } from "../terminal-session";
import { beginLayoutResize } from "../../../lib/layout-resize";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  channels: [] as Array<{ onmessage: (event: unknown) => void }>,
  terminals: [] as Array<{
    cols: number; rows: number; options: Record<string, unknown>;
    emit: (data: string) => void; write: ReturnType<typeof vi.fn>;
    dispose: ReturnType<typeof vi.fn>; focus: ReturnType<typeof vi.fn>;
    paste: ReturnType<typeof vi.fn>; key: (event: Partial<KeyboardEvent>) => boolean;
    title: (title: string) => void; selection: string;
    textarea: HTMLTextAreaElement;
  }>,
  dimensions: { cols: 100, rows: 30 },
  fit: vi.fn(),
  optionWrites: vi.fn(),
  atOpen: [] as Record<string, unknown>[],
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
  Channel: class {
    onmessage = (_event: unknown) => {};
    constructor() { mocks.channels.push(this); }
  },
}));
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    options: Record<string, unknown>;
    write = vi.fn();
    textarea = document.createElement("textarea");
    focus = vi.fn(() => this.textarea.focus());
    dispose = vi.fn();
    blur = vi.fn(() => this.textarea.blur());
    paste = vi.fn();
    selection = "";
    private input = (_data: string) => {};
    private keys = (_event: KeyboardEvent) => true;
    private titles = (_title: string) => {};
    constructor(options: Record<string, unknown>) {
      this.options = new Proxy(options, {
        set(target, key, value) {
          mocks.optionWrites(key, value);
          return Reflect.set(target, key, value);
        },
      });
      mocks.terminals.push(this);
    }
    loadAddon() {}
    open(host: HTMLElement) { mocks.atOpen.push({ ...this.options }); host.append(this.textarea); }
    resize(cols: number, rows: number) { this.cols = cols; this.rows = rows; }
    onData(callback: (data: string) => void) { this.input = callback; return { dispose: vi.fn() }; }
    onTitleChange(callback: (title: string) => void) { this.titles = callback; return { dispose: vi.fn() }; }
    attachCustomKeyEventHandler(handler: (event: KeyboardEvent) => boolean) { this.keys = handler; }
    hasSelection() { return this.selection.length > 0; }
    getSelection() { return this.selection; }
    emit(data: string) { this.input(data); }
    key(event: Partial<KeyboardEvent>) {
      return this.keys({ type: "keydown", altKey: false, ctrlKey: false, metaKey: false, shiftKey: false,
        preventDefault: () => {}, ...event } as KeyboardEvent);
    }
    title(title: string) { this.titles(title); }
  },
}));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    proposeDimensions() { return mocks.dimensions; }
    fit() {
      mocks.fit();
      Object.assign(mocks.terminals[mocks.terminals.length - 1], mocks.dimensions);
    }
  },
}));

const appearance: TerminalAppearance = {
  theme: { background: "#000000", foreground: "#ffffff" },
  fontFamily: "monospace", fontSize: 12, lineHeight: 1.2,
  cursorStyle: "block", cursorBlink: true, scrollback: 3000,
};
const sessions: TerminalSession[] = [];
const states: SessionState[] = [];
function create(width = 500, height = 250, clipboard: () => Promise<string | null> = async () => null, canFocus?: () => boolean) {
  const host = document.createElement("div");
  Object.defineProperties(host, {
    clientWidth: { get: () => width * mocks.dimensions.cols / 100 },
    clientHeight: { get: () => height * mocks.dimensions.rows / 30 },
  });
  document.body.append(host);
  const session = new TerminalSession(host, "/fixture", (state) => states.push(state), appearance, clipboard, canFocus);
  sessions.push(session);
  return session;
}
const calls = (command: string) => mocks.invoke.mock.calls.filter(([name]) => name === command);
const flush = () => vi.advanceTimersByTimeAsync(32);

beforeEach(() => {
  vi.useFakeTimers();
  mocks.invoke.mockReset();
  mocks.channels.length = 0;
  mocks.terminals.length = 0;
  mocks.fit.mockClear();
  mocks.optionWrites.mockClear();
  mocks.atOpen.length = 0;
  mocks.dimensions = { cols: 100, rows: 30 };
  states.length = 0;
  mocks.invoke.mockImplementation((command: string) => Promise.resolve(
    command === "terminal_open" ? { id: "pty-fixture", cwd: "/fixture", shell: "/bin/sh" } : undefined,
  ));
});
afterEach(() => {
  for (const session of sessions.splice(0)) session.dispose();
  document.body.querySelectorAll("div").forEach((element) => element.remove());
});

test("passes UTF-8 bytes unchanged and keeps the same PTY when hidden", async () => {
  const session = create();
  session.setVisible(true);
  await flush();
  expect(calls("terminal_open")).toHaveLength(1);
  mocks.channels[0].onmessage(new Uint8Array([0xe4, 0xbd]).buffer);
  mocks.channels[0].onmessage(new Uint8Array([0xa0]).buffer);
  expect(mocks.terminals[0].write.mock.calls).toEqual([[new Uint8Array([0xe4, 0xbd])], [new Uint8Array([0xa0])]]);
  session.setVisible(false);
  expect(mocks.terminals[0].options.disableStdin).toBe(true);
  expect(calls("terminal_close")).toHaveLength(0);
  session.setVisible(true);
  await flush();
  expect(calls("terminal_open")).toHaveLength(1);
  expect(mocks.terminals[0].options.disableStdin).toBe(false);
});

test("initial appearance is applied before the renderer opens", () => {
  create();
  expect(mocks.atOpen[0]).toMatchObject({
    fontFamily: "monospace", fontSize: 12, lineHeight: 1.2,
    theme: appearance.theme, cursorStyle: "block", scrollback: 3000,
  });
});

test("palette and cursor updates neither reset font options nor invalidate fitted geometry", async () => {
  const session = create();
  session.setVisible(true);
  await flush();
  mocks.optionWrites.mockClear();
  mocks.fit.mockClear();
  const next = { ...appearance, theme: { background: "#282a36" }, cursorBlink: false };
  session.setAppearance(next);
  await flush();
  expect(mocks.optionWrites.mock.calls.map(([key]) => key)).toEqual(["theme", "cursorBlink"]);
  expect(mocks.fit).not.toHaveBeenCalled();
  session.setVisible(false);
  session.setVisible(true);
  await flush();
  expect(mocks.fit).not.toHaveBeenCalled();
});

test("font-size changes preserve the palette and refit geometry once", async () => {
  const session = create();
  session.setVisible(true);
  await flush();
  mocks.optionWrites.mockClear();
  mocks.fit.mockClear();
  session.setAppearance({ ...appearance, theme: { ...appearance.theme }, fontSize: 16 });
  await flush();
  expect(mocks.optionWrites.mock.calls.map(([key]) => key)).toEqual(["fontSize"]);
  expect(mocks.fit).toHaveBeenCalledOnce();
});

test("switching a same-sized terminal back on reuses fitted cell geometry", async () => {
  const session = create();
  session.setVisible(true);
  await flush();
  mocks.fit.mockClear();
  session.setVisible(false);
  session.setVisible(true);
  await flush();
  expect(mocks.fit).not.toHaveBeenCalled();
  expect(calls("terminal_resize")).toHaveLength(0);
});

test("visible unfocused split panes resize without stealing keyboard focus", async () => {
  const session = create();
  session.setVisible(true, false);
  await flush();
  expect(mocks.terminals[0].focus).not.toHaveBeenCalled();
  session.setVisible(true, true);
  await flush();
  expect(mocks.terminals[0].focus).toHaveBeenCalledOnce();
});

test("a queued autofocus can be vetoed without preventing an explicit user focus", async () => {
  const session = create(500, 250, async () => null, () => false);
  session.setVisible(true);
  await flush();
  expect(mocks.terminals[0].focus).not.toHaveBeenCalled();
  session.focus();
  expect(mocks.terminals[0].focus).toHaveBeenCalledOnce();
});

test("repeated shell focus/title reports do not notify unchanged UI state", async () => {
  create();
  await flush();
  const count = states.length;
  mocks.terminals[0].title(" task ");
  mocks.terminals[0].title("task");
  expect(states).toHaveLength(count + 1);
  expect(states.at(-1)?.title).toBe("task");
  mocks.terminals[0].title(" ");
  expect(states.at(-1)?.title).toBeUndefined();
});

test("a renderer created in a hidden group fits its real size on first activation", async () => {
  mocks.dimensions = { cols: 0, rows: 30 };
  const session = create();
  await flush();
  expect(calls("terminal_open")[0][1]).toMatchObject({ cols: 80, rows: 24 });
  mocks.dimensions = { cols: 100, rows: 30 };
  session.setVisible(true);
  await flush();
  expect(calls("terminal_resize")).toEqual([
    ["terminal_resize", { id: "pty-fixture", cols: 100, rows: 30 }],
  ]);
  expect(mocks.terminals[0].focus).toHaveBeenCalledOnce();
});

test("layout resizing defers all intermediate fits until the final geometry", async () => {
  const session = create();
  session.setVisible(true);
  await flush();
  mocks.fit.mockClear();
  const release = beginLayoutResize();
  try {
    for (const rows of [31, 35, 40]) {
      mocks.dimensions = { cols: 100, rows };
      session.scheduleFit({ immediate: true });
      await flush();
    }
    expect(mocks.fit).not.toHaveBeenCalled();
    expect(calls("terminal_resize")).toHaveLength(0);
  } finally {
    release();
  }
  session.scheduleFit({ immediate: true });
  await flush();
  expect(mocks.fit).toHaveBeenCalledOnce();
  expect(calls("terminal_resize")).toEqual([
    ["terminal_resize", { id: "pty-fixture", cols: 100, rows: 40 }],
  ]);
});

test("a fit queued before pointerdown cannot run inside the resize gesture", async () => {
  const session = create();
  session.setVisible(true);
  await flush();
  mocks.fit.mockClear();
  mocks.dimensions = { cols: 100, rows: 45 };
  session.scheduleFit({ immediate: true });
  const release = beginLayoutResize();
  try {
    await flush();
    expect(mocks.fit).not.toHaveBeenCalled();
    expect(calls("terminal_resize")).toHaveLength(0);
  } finally {
    release();
  }
  session.scheduleFit({ immediate: true });
  await flush();
  expect(mocks.fit).toHaveBeenCalledOnce();
});

test("a size change while PTY open is pending is still sent to the backend", async () => {
  let opened!: (result: unknown) => void;
  mocks.invoke.mockImplementation((command: string) => command === "terminal_open"
    ? new Promise((resolve) => { opened = resolve; })
    : Promise.resolve());
  const session = create();
  session.setVisible(true);
  await flush();
  mocks.dimensions = { cols: 120, rows: 40 };
  session.scheduleFit();
  await flush();
  expect(calls("terminal_resize")).toHaveLength(0);
  opened({ id: "pty-fixture", cwd: "/fixture", shell: "/bin/sh" });
  await flush();
  expect(calls("terminal_resize")).toEqual([
    ["terminal_resize", { id: "pty-fixture", cols: 120, rows: 40 }],
  ]);
});

test("a busy PTY receives only the newest pending size, not a queue of drag positions", async () => {
  const session = create();
  session.setVisible(true);
  await flush();
  const pending: (() => void)[] = [];
  mocks.invoke.mockImplementation((command: string) => command === "terminal_resize"
    ? new Promise<void>((resolve) => pending.push(resolve))
    : Promise.resolve());
  for (const rows of [31, 32, 33, 34, 40]) {
    mocks.dimensions = { cols: 100, rows };
    session.scheduleFit();
    await flush();
  }
  expect(calls("terminal_resize")).toHaveLength(1);
  pending.shift()!();
  await flush();
  expect(calls("terminal_resize")).toEqual([
    ["terminal_resize", { id: "pty-fixture", cols: 100, rows: 31 }],
    ["terminal_resize", { id: "pty-fixture", cols: 100, rows: 40 }],
  ]);
  pending.shift()!();
  await flush();
  expect(calls("terminal_resize")).toHaveLength(2);
});

test("dragging back to the in-flight size drops an obsolete pending size", async () => {
  const session = create();
  session.setVisible(true);
  await flush();
  let finish!: () => void;
  mocks.invoke.mockImplementation((command: string) => command === "terminal_resize"
    ? new Promise<void>((resolve) => { finish = resolve; })
    : Promise.resolve());
  for (const rows of [31, 40, 31]) {
    mocks.dimensions = { cols: 100, rows };
    session.scheduleFit();
    await flush();
  }
  finish();
  await flush();
  expect(calls("terminal_resize")).toHaveLength(1);
});

test("continuous width changes coalesce the expensive scrollback reflow", async () => {
  const session = create();
  session.setVisible(true);
  await flush();
  mocks.fit.mockClear();
  for (const cols of [90, 80, 70, 60]) {
    mocks.dimensions = { cols, rows: 30 };
    session.scheduleFit({ widthChanged: true });
    await vi.advanceTimersByTimeAsync(16);
  }
  expect(mocks.fit).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(100);
  expect(mocks.fit).toHaveBeenCalledOnce();
  expect(calls("terminal_resize").at(-1)).toEqual([
    "terminal_resize", { id: "pty-fixture", cols: 60, rows: 30 },
  ]);
});

test("a committed resize flushes a pending width fit without waiting for debounce", async () => {
  const session = create();
  session.setVisible(true);
  await flush();
  mocks.fit.mockClear();
  mocks.dimensions = { cols: 70, rows: 40 };
  session.scheduleFit({ widthChanged: true });
  session.scheduleFit({ immediate: true });
  await vi.advanceTimersByTimeAsync(20);
  expect(mocks.fit).toHaveBeenCalledOnce();
  await vi.advanceTimersByTimeAsync(100);
  expect(mocks.fit).toHaveBeenCalledOnce();
});

test("serializes separate input events instead of reordering writes", async () => {
  create().setVisible(true);
  await flush();
  let finishFirst!: () => void;
  mocks.invoke.mockImplementation((command: string) => command === "terminal_write"
    ? new Promise<void>((resolve) => { finishFirst = resolve; })
    : Promise.resolve());
  mocks.terminals[0].emit("first");
  mocks.terminals[0].emit("second");
  await flush();
  expect(calls("terminal_write")).toHaveLength(1);
  expect(calls("terminal_write")[0][1]).toEqual({ id: "pty-fixture", data: "first" });
  finishFirst();
  await flush();
  expect(calls("terminal_write")[1][1]).toEqual({ id: "pty-fixture", data: "second" });
  finishFirst();
});

test("splits a large paste without corrupting surrogate pairs", async () => {
  create().setVisible(true);
  await flush();
  const data = "x".repeat(4095) + "😀".repeat(5000);
  mocks.terminals[0].emit(data);
  await flush();
  const chunks = calls("terminal_write").map(([, args]) => (args as { data: string }).data);
  expect(chunks.join("")).toBe(data);
  expect(chunks.every((chunk) => new TextEncoder().encode(chunk).length <= 64 * 1024)).toBe(true);
});

test("spawns the PTY at the fitted size, not 80x24", async () => {
  create().setVisible(true);
  await flush();
  expect(calls("terminal_open")[0][1]).toMatchObject({ cols: 100, rows: 30 });
  // The open already used this size; no resize round-trip follows.
  expect(calls("terminal_resize")).toHaveLength(0);
});

test("does not resize a hidden or zero-sized host", async () => {
  const session = create(0, 0);
  session.setVisible(true);
  await flush();
  expect(mocks.fit).not.toHaveBeenCalled();
  expect(calls("terminal_resize")).toHaveLength(0);
  session.setVisible(false);
  session.scheduleFit();
  await flush();
  expect(calls("terminal_resize")).toHaveLength(0);
});

test("closes an orphan PTY if opening completes after unmount", async () => {
  let opened!: (value: unknown) => void;
  mocks.invoke.mockImplementation((command: string) => command === "terminal_open"
    ? new Promise((resolve) => { opened = resolve; })
    : Promise.resolve());
  const session = create();
  session.dispose();
  session.dispose();
  opened({ id: "pty-late", cwd: "/fixture", shell: "/bin/sh" });
  await flush();
  expect(calls("terminal_close")).toEqual([["terminal_close", { id: "pty-late" }]]);
  expect(mocks.terminals[0].dispose).toHaveBeenCalledOnce();
});

test("an exit disables input and preserves its exit status", async () => {
  create().setVisible(true);
  await flush();
  mocks.channels[0].onmessage({ type: "exit", code: 7, signal: null });
  mocks.terminals[0].emit("not sent");
  await flush();
  expect(calls("terminal_write")).toHaveLength(0);
  expect(states[states.length - 1]).toMatchObject({ status: "exited", code: 7 });
  expect(mocks.terminals[0].options.disableStdin).toBe(true);
});

test("appearance applies to the renderer and an identical one is a no-op", async () => {
  const session = create();
  const options = mocks.terminals[0].options;
  expect(options).toMatchObject({ fontSize: 12, cursorStyle: "block", scrollback: 3000 });
  session.setAppearance({ ...appearance, fontSize: 15, cursorStyle: "bar", theme: { background: "#fdf6e3" } });
  expect(options).toMatchObject({ fontSize: 15, cursorStyle: "bar", theme: { background: "#fdf6e3" } });
  options.fontSize = 99;
  session.setAppearance({ ...appearance, fontSize: 15, cursorStyle: "bar", theme: { background: "#fdf6e3" } });
  expect(options.fontSize).toBe(99);
});

test("the shell's OSC title reaches the tab state", async () => {
  create().setVisible(true);
  await flush();
  mocks.terminals[0].title("vim notes.md");
  expect(states[states.length - 1]).toMatchObject({ status: "running", title: "vim notes.md" });
});

test("copy and paste shortcuts stay out of the shell; plain Ctrl+C still reaches it", async () => {
  const clipboard = vi.fn(async () => "echo pasted");
  const writeText = vi.fn(async () => {});
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
  create(500, 250, clipboard).setVisible(true);
  await flush();
  const terminal = mocks.terminals[0];
  // Linux/Windows layer: Ctrl+Shift+C/V. (jsdom is not a Mac.)
  expect(terminal.key({ key: "c", ctrlKey: true })).toBe(true);
  terminal.selection = "selected text";
  expect(terminal.key({ key: "C", ctrlKey: true, shiftKey: true })).toBe(false);
  expect(writeText).toHaveBeenCalledWith("selected text");
  expect(terminal.key({ key: "V", ctrlKey: true, shiftKey: true })).toBe(false);
  await flush();
  expect(clipboard).toHaveBeenCalledOnce();
  expect(terminal.paste).toHaveBeenCalledWith("echo pasted");
});

test("an error after a clean exit does not overwrite the exit status", async () => {
  create().setVisible(true);
  await flush();
  mocks.channels[0].onmessage({ type: "exit", code: 0, signal: null });
  mocks.channels[0].onmessage({ type: "error", message: "late" });
  expect(states[states.length - 1]).toMatchObject({ status: "exited", code: 0 });
});

function deferredClipboard() {
  let resolve!: (text: string | null) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<string | null>((yes, no) => { resolve = yes; reject = no; });
  return { read: () => promise, resolve, reject };
}

test.each([
  ["pane", "text"], ["pane", "null"], ["pane", "error"],
  ["input", "text"], ["input", "null"], ["input", "error"],
] as const)("delayed paste preserves focus moved to another %s on clipboard %s", async (target, outcome) => {
  const clipboard = deferredClipboard();
  const session = create(500, 250, clipboard.read);
  session.setVisible(true);
  await flush();
  const terminal = mocks.terminals[0];
  let destination: HTMLElement;
  if (target === "pane") {
    create().setVisible(true);
    await flush();
    destination = mocks.terminals[1].textarea;
  } else {
    const wrapper = document.createElement("div");
    destination = document.createElement("input");
    wrapper.append(destination);
    document.body.append(wrapper);
  }
  terminal.textarea.focus();
  const paste = session.pasteFromClipboard();
  destination.focus();
  expect(document.activeElement).toBe(destination);
  if (outcome === "error") clipboard.reject(new Error("clipboard unavailable"));
  else clipboard.resolve(outcome === "text" ? "echo pasted" : null);
  await paste;
  expect(document.activeElement).toBe(destination);
  if (outcome === "text") expect(terminal.paste).toHaveBeenCalledWith("echo pasted");
  else expect(terminal.paste).not.toHaveBeenCalled();
});

test("delayed paste keeps the terminal focused when focus has not moved", async () => {
  const clipboard = deferredClipboard();
  const session = create(500, 250, clipboard.read);
  session.setVisible(true);
  await flush();
  const terminal = mocks.terminals[0];
  expect(document.activeElement).toBe(terminal.textarea);
  const paste = session.pasteFromClipboard();
  clipboard.resolve("echo pasted");
  await paste;
  expect(terminal.paste).toHaveBeenCalledWith("echo pasted");
  expect(document.activeElement).toBe(terminal.textarea);
});

test.each(["body", "terminal", "input"] as const)(
  "explicit menu paste handles focus on %s after the portalled menu closes",
  async (destination) => {
    const clipboard = deferredClipboard();
    // TerminalView's automatic-focus guard rejects a menu owner; explicit
    // menu paste must still restore the terminal after that menu is removed.
    const session = create(500, 250, clipboard.read, () => false);
    session.setVisible(true);
    await flush();
    const terminal = mocks.terminals[0];
    terminal.textarea.focus();
    session.setVisible(true, false);
    const menu = document.createElement("div");
    menu.setAttribute("role", "menu");
    const item = document.createElement("button");
    item.setAttribute("role", "menuitem");
    menu.append(item);
    document.body.append(menu);
    item.focus();
    expect(document.activeElement).toBe(item);
    const paste = session.pasteFromClipboard();
    menu.remove();
    expect(document.activeElement).toBe(document.body);
    let expected: HTMLElement = terminal.textarea;
    if (destination === "terminal") terminal.textarea.focus();
    if (destination === "input") {
      const wrapper = document.createElement("div");
      const input = document.createElement("input");
      wrapper.append(input);
      document.body.append(wrapper);
      input.focus();
      expected = input;
    }
    clipboard.resolve("echo pasted");
    await paste;
    expect(terminal.paste).toHaveBeenCalledWith("echo pasted");
    expect(document.activeElement).toBe(expected);
  },
);
