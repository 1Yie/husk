import { TerminalSession, type TerminalAppearance } from "../../src/features/terminal/terminal-session";
import { probeHidden, setShellTheme, shell } from "./tauri";
import "../../src/index.css";
import "../../src/features/terminal/terminal.css";

const light = new URLSearchParams(location.search).get("theme") === "light";
document.documentElement.classList.toggle("dark", !light);
const appearance: TerminalAppearance = {
  theme: {
    background: light ? "#f6f8fa" : "#101820",
    foreground: light ? "#24292f" : "#d4d4d4", cursor: "#c3a5ff",
    red: "#bf616a", brightRed: "#d08770",
  },
  fontFamily: "monospace", fontSize: 14, lineHeight: 1.2,
  cursorStyle: "block", cursorBlink: false, scrollback: 1000,
};

const host = document.getElementById("terminal")!;
host.parentElement!.style.setProperty("--terminal-bg", appearance.theme.background!);
const state = { current: "opening" };
const session = new TerminalSession(host, "/fixture", (next) => { state.current = next.status; }, appearance);
session.setVisible(true);
Object.assign(window, {
  __terminalColors: {
    shell, state,
    hide: () => session.setVisible(false),
    show: () => session.setVisible(true),
    probeHidden,
    setShellTheme,
    changeFont: () => session.setAppearance({ ...appearance, fontSize: 16, cursorStyle: "bar" }),
  },
});
window.addEventListener("beforeunload", () => session.dispose(), { once: true });
