"""Optional Linux check: python3 + PyGObject, GTK 3, WebKit2 4.1, a display."""
import json
import argparse
import mimetypes
from pathlib import Path
from urllib.parse import unquote, urlsplit
import gi

gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import Gtk, WebKit2, GLib, Gio

parser = argparse.ArgumentParser()
parser.add_argument("url")
parser.add_argument("--asset-root", type=Path)
parser.add_argument("--reduced-motion", action="store_true")
args = parser.parse_args()
reduced_motion_requested = args.reduced_motion
if reduced_motion_requested:
    Gtk.Settings.get_default().set_property("gtk-enable-animations", False)

result = None
resource_failures = []
manager = WebKit2.UserContentManager()
manager.register_script_message_handler("terminalCheck")


def received(_manager, message):
    global result
    result = json.loads(message.get_js_value().to_string())
    print(json.dumps(result, indent=2), flush=True)
    Gtk.main_quit()


manager.connect("script-message-received::terminalCheck", received)
manager.add_script(WebKit2.UserScript.new(r"""
(async () => {
  const errors = [];
  addEventListener("error", (event) => { if (event.message) errors.push(event.message); });
  addEventListener("unhandledrejection", (event) => errors.push(String(event.reason)));
  const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const until = async (test) => {
    const deadline = performance.now() + 10000;
    while (!test()) {
      if (performance.now() > deadline) throw new Error("Timed out waiting for real page state");
      await wait(25);
    }
  };
  const panel = () => document.querySelector(".terminal-panel");
  const surface = () => document.querySelector(".terminal-panel__surface");
  const composer = () => document.querySelector("[data-terminal-composer]");
  const toggle = () => document.querySelector("#session-navigation button[aria-pressed]");
  const click = (node) => { node.focus(); node.click(); };
  const metrics = () => window.__terminalMetrics = { start: performance.now(), ipc: [], chatCommits: [], fits: [] };
  try {
    await until(() => panel() && composer());
    await document.fonts.ready;
    await wait(1000);
    const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;
    const runs = [];
    for (const opening of [true, false, true, false]) {
      const data = metrics();
      let target = opening ? 0 : panel().getBoundingClientRect().height;
      const frames = [];
      const timers = [];
      let previousTimer = performance.now();
      const timer = setInterval(() => {
        const now = performance.now();
        timers.push(now - previousTimer);
        previousTimer = now;
      }, 16);
      let active = true;
      const sample = () => {
        const beforeRead = performance.now();
        const box = panel().getBoundingClientRect();
        const inner = surface().getBoundingClientRect();
        const followGap = Math.abs(composer().getBoundingClientRect().bottom - box.top);
        frames.push({ at: performance.now() - data.start, height: box.height,
          geometryGap: Math.abs(box.height - inner.height),
          followGap, readMs: performance.now() - beforeRead });
        if (active) requestAnimationFrame(sample);
      };
      requestAnimationFrame(sample);
      click(toggle());
      await wait(0);
      if (opening) target = Number.parseFloat(panel().style.height);
      await wait(1000);
      active = false;
      clearInterval(timer);
      timers.sort((a, b) => a - b);
      const middle = frames.filter((frame) => frame.height > 2 && frame.height < target - 2);
      runs.push({ opening, intermediateFrames: middle.length,
        maxGeometryGapPx: Math.max(...frames.map((frame) => frame.geometryGap)),
        maxFollowGapPx: Math.max(...frames.map((frame) => frame.followGap)),
        maxLayoutReadMs: Math.max(...frames.map((frame) => frame.readMs)),
        p95TimerMs: timers[Math.floor(timers.length * .95)],
        chatCommits: data.chatCommits.length,
        hasFocus: document.hasFocus(), visibility: document.visibilityState,
        settledMs: frames.find((frame) => opening ? frame.height >= target - 0.5 : frame.height < 0.5)?.at });
    }
    click(toggle());
    await until(() => document.querySelector(".xterm-rows"));
    await wait(1000);
    const screen = document.querySelector(".terminal-panel__screen").getBoundingClientRect();
    document.querySelector(".terminal-panel__screen").dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true, button: 2, clientX: screen.left + 30, clientY: screen.top + 30,
    }));
    await until(() => document.querySelector('[role="menu"]'));
    const screenMenuItems = [...document.querySelectorAll('[role="menu"] [role="menuitem"]')];
    const closeItem = screenMenuItems.find((node) => node.textContent.trim() === "关闭终端");
    const closeColors = () => ({
      text: getComputedStyle(closeItem).color,
      icon: getComputedStyle(closeItem.querySelector("svg")).color,
    });
    const normalCloseColors = closeColors();
    closeItem.focus();
    const screenMenuStyle = {
      items: screenMenuItems.length,
      icons: screenMenuItems.filter((node) => node.querySelector('svg[aria-hidden="true"]')).length,
      normal: normalCloseColors, focused: closeColors(),
    };
    (document.activeElement ?? document.body).dispatchEvent(new KeyboardEvent("keydown", {
      key: "J", ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true,
    }));
    await until(() => panel().dataset.open === "false" && !document.querySelector('[role="menu"]'));
    await wait(400);
    click(toggle());
    await wait(1200);
    const TerminalSession = window.__terminalTestSession;
    const originalFit = TerminalSession.prototype.fitNow;
    TerminalSession.prototype.fitNow = function () {
      const start = performance.now();
      try { return originalFit.call(this); } finally {
        window.__terminalMetrics.fits.push({ at: performance.now() - window.__terminalMetrics.start,
          ms: performance.now() - start });
      }
    };
    const data = metrics();
    const handle = document.querySelector('[role="separator"][aria-label="调整终端高度"]');
    // Synthetic input exercises the real React pointer handlers. Only capture
    // is bypassed: WebKit correctly rejects capture for synthetic pointer IDs.
    handle.setPointerCapture = () => {};
    handle.hasPointerCapture = () => false;
    const box = handle.getBoundingClientRect();
    const x = box.left + box.width / 2;
    const y = box.top + 3;
    let previous = performance.now();
    let sampling = true;
    const gaps = [];
    let maxGeometryGap = 0;
    let maxFollowGap = 0;
    const sample = () => {
      const now = performance.now();
      gaps.push(now - previous);
      previous = now;
      const rect = panel().getBoundingClientRect();
      maxGeometryGap = Math.max(maxGeometryGap, Math.abs(rect.height - surface().getBoundingClientRect().height));
      maxFollowGap = Math.max(maxFollowGap, Math.abs(composer().getBoundingClientRect().bottom - rect.top));
      if (sampling) requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
    const pointer = (type, clientY) => handle.dispatchEvent(new PointerEvent(type, {
      bubbles: true, pointerId: 21, button: 0, buttons: type === "pointerup" ? 0 : 1, clientX: x, clientY,
    }));
    pointer("pointerdown", y);
    for (const [from, to] of [[y, y - 180], [y - 180, y + 40], [y + 40, y - 120]])
      for (let n = 1; n <= 40; n++) {
        pointer("pointermove", from + (to - from) * n / 40);
        await wait(8);
      }
    const released = performance.now() - data.start;
    pointer("pointerup", y - 120);
    sampling = false;
    await wait(700);
    TerminalSession.prototype.fitNow = originalFit;
    gaps.sort((a, b) => a - b);
    const drag = { fitsWhileHeld: data.fits.filter((fit) => fit.at < released).length,
      resizeRequests: data.ipc.filter((call) => call.command === "terminal_resize").length,
      p95FrameMs: gaps[Math.floor(gaps.length * 0.95)], maxGeometryGapPx: maxGeometryGap, maxFollowGapPx: maxFollowGap };
    const splitMetrics = metrics();
    const originalHost = document.querySelector('[data-pane-key="1"] .terminal-panel__screen');
    click(document.querySelector('button[aria-label="左右分屏"]'));
    await until(() => document.querySelector('[data-pane-key="2"] .xterm-rows'));
    await wait(300);
    document.querySelector('[data-pane-key="1"]').focus();
    click(document.querySelector('button[aria-label="上下分屏"]'));
    await until(() => document.querySelector('[data-pane-key="3"] .xterm-rows'));
    await wait(300);
    const retainedHost = originalHost === document.querySelector('[data-pane-key="1"] .terminal-panel__screen');
    const boxes = [...document.querySelectorAll('.terminal-panel__group:not([hidden]) [data-pane-key]')]
      .map((node) => ({ key: Number(node.dataset.paneKey), ...node.getBoundingClientRect().toJSON() }));
    const top = boxes.find((box) => box.key === 1);
    const bottom = boxes.find((box) => box.key === 3);
    click(document.querySelector('button[aria-label="新建终端"]'));
    await until(() => document.querySelector('[data-pane-key="4"] .xterm-rows'));
    await wait(400);
    const beforeSwitch = { opens: splitMetrics.ipc.filter((call) => call.command === "terminal_open").length,
      resizes: splitMetrics.ipc.filter((call) => call.command === "terminal_resize").length };
    for (const index of [0, 1, 0, 1]) {
      click(document.querySelectorAll('.terminal-panel [role="tab"]')[index]);
      await wait(300);
    }
    const warmSwitch = {
      opens: splitMetrics.ipc.filter((call) => call.command === "terminal_open").length - beforeSwitch.opens,
      resizes: splitMetrics.ipc.filter((call) => call.command === "terminal_resize").length - beforeSwitch.resizes,
    };
    const tab = document.querySelector('.terminal-panel [role="tab"]');
    const point = tab.getBoundingClientRect();
    tab.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, button: 2, clientX: point.left + 20, clientY: point.top + 10 }));
    await until(() => [...document.querySelectorAll('[role="menuitem"]')].some((node) => node.textContent.trim() === "重命名"));
    [...document.querySelectorAll('[role="menuitem"]')].find((node) => node.textContent.trim() === "重命名").click();
    await until(() => document.querySelector('input[aria-label="终端标签名称"]'));
    const rename = document.querySelector('input[aria-label="终端标签名称"]');
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(rename, "build");
    rename.dispatchEvent(new Event("input", { bubbles: true }));
    await wait(0);
    rename.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await until(() => document.querySelector('.terminal-panel [role="tab"]')?.textContent.includes("build"));
    const renamed = document.querySelector('.terminal-panel [role="tab"]');
    renamed.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, button: 2, clientX: point.left + 20, clientY: point.top + 10 }));
    await until(() => [...document.querySelectorAll('[role="menuitem"]')].some((node) => node.textContent.trim() === "关闭全部终端"));
    const tabMenuCompact = [...document.querySelectorAll('[role="menuitem"]')].every((node) => getComputedStyle(node).fontSize === "12px");
    [...document.querySelectorAll('[role="menuitem"]')].find((node) => node.textContent.trim() === "关闭全部终端").click();
    await until(() => panel().dataset.open === "false" && !document.querySelector('[role="menu"]'));
    const split = { panes: boxes.length, retainedHost, nestedGeometry: Math.abs(top.bottom - bottom.top) < 1,
      renamed: true, tabMenuCompact, openedPtys: splitMetrics.ipc.filter((call) => call.command === "terminal_open").length,
      closedPtys: splitMetrics.ipc.filter((call) => call.command === "terminal_close").length };
    window.webkit.messageHandlers.terminalCheck.postMessage(JSON.stringify({
      engine: "WebKitGTK", reducedMotion, runs, menuDismissedOnShortcut: true,
      drag, split, warmSwitch, screenMenuStyle,
      errors,
    }));
  } catch (error) {
    window.webkit.messageHandlers.terminalCheck.postMessage(JSON.stringify({ error: String(error.stack ?? error), errors }));
  }
})();
""", WebKit2.UserContentInjectedFrames.TOP_FRAME, WebKit2.UserScriptInjectionTime.END, None, None))
view = WebKit2.WebView.new_with_user_content_manager(manager)
if args.asset_root:
    asset_root = args.asset_root.resolve()

    def asset_requested(request):
        try:
            uri = urlsplit(request.get_uri())
            if uri.hostname != "localhost":
                raise ValueError("fixture host must be localhost")
            resource = (asset_root / unquote(uri.path).lstrip("/")).resolve()
            resource.relative_to(asset_root)
            stream = Gio.File.new_for_path(str(resource)).read(None)
            mime = mimetypes.guess_type(str(resource))[0] or "application/octet-stream"
            request.finish(stream, resource.stat().st_size, mime)
        except (OSError, ValueError, GLib.Error) as error:
            request.finish_error(GLib.Error.new_literal(
                Gio.io_error_quark(), f"Fixture resource unavailable: {error}", Gio.IOErrorEnum.NOT_FOUND))

    context = view.get_context()
    context.register_uri_scheme("husk-terminal-test", asset_requested)
    security = context.get_security_manager()
    security.register_uri_scheme_as_secure("husk-terminal-test")
    security.register_uri_scheme_as_cors_enabled("husk-terminal-test")


def resource_started(_view, resource, _request):
    def failed(item, error):
        resource_failures.append({"uri": item.get_uri(), "message": error.message, "code": error.code})
    resource.connect("failed", failed)


view.connect("resource-load-started", resource_started)
view.get_settings().set_hardware_acceleration_policy(WebKit2.HardwareAccelerationPolicy.ALWAYS)
window = Gtk.Window()
window.set_focus_on_map(False)
window.set_default_size(1280, 840)
window.add(view)
window.show_all()
view.load_uri(args.url)
GLib.timeout_add_seconds(35, Gtk.main_quit)
Gtk.main()
window.destroy()
if resource_failures:
    print(json.dumps({"resourceLoadFailures": resource_failures}, indent=2), flush=True)
assert result and "error" not in result, result
assert not resource_failures, resource_failures
assert not result["errors"], result
assert result["screenMenuStyle"]["items"] == result["screenMenuStyle"]["icons"] == 6, result
for state in ("normal", "focused"):
    assert result["screenMenuStyle"][state] == {
        "text": "rgb(220, 38, 38)", "icon": "rgb(220, 38, 38)"
    }, result
assert not reduced_motion_requested or result["reducedMotion"], result
for run in result["runs"]:
    if result["reducedMotion"]:
        assert run["intermediateFrames"] == 0, run
    else:
        assert run["intermediateFrames"] >= 2, run
    assert run["settledMs"] < 900, run
    assert run["maxGeometryGapPx"] < 1, run
    assert run["maxFollowGapPx"] < 2, run
assert result["drag"]["fitsWhileHeld"] == 0, result
assert result["drag"]["resizeRequests"] == 1, result
assert result["drag"]["maxGeometryGapPx"] < 1, result
assert result["drag"]["maxFollowGapPx"] < 2, result
assert result["split"]["panes"] == 3, result
assert result["split"]["retainedHost"] and result["split"]["nestedGeometry"], result
assert result["split"]["renamed"] and result["split"]["tabMenuCompact"], result
assert result["split"]["openedPtys"] == 3 and result["split"]["closedPtys"] == 4, result
assert result["warmSwitch"]["opens"] == 0 and result["warmSwitch"]["resizes"] == 0, result
