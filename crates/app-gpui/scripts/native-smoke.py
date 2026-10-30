#!/usr/bin/env python3
"""Prepare isolated sessions with a loopback model and launch the native window."""

import argparse
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


REPLY = """## 原生界面验收

保留清晰的会话导航，将注意力留给当前任务。

- 左侧按会话和项目组织，工作区切换保持独立状态。
- 聊天内容使用原生 Markdown，支持中文、列表和代码块。
- 右侧展示真实文件改动及增删统计。
- 终端使用独立等宽字体，隐藏后保留进程。

> 输入框固定在聊天区底部，模型和权限均通过真实宿主接口设置。

### 验证结果

1. 本段文字通过本地 SSE 服务进入真实会话。
2. 完成后重新打开窗口，验收历史恢复。
3. 使用隔离配置和数据目录，不读取生产账号。

```rust
fn main() {
    println!("你好，Husk");
}
```

此页面是本地验收数据，不是外部模型的运行结果。
"""


class LocalModel(BaseHTTPRequestHandler):
    def log_message(self, format: str, *args) -> None:
        pass

    def do_POST(self):
        try:
            self.respond()
        except (BrokenPipeError, ConnectionResetError):
            pass

    def respond(self):
        try:
            length = int(self.headers.get("Content-Length", "0"))
            if length < 0:
                raise ValueError("negative Content-Length")
            request = json.loads(self.rfile.read(length))
        except ValueError:
            self.send_error(400, "Invalid JSON request")
            return
        if not isinstance(request, dict):
            self.send_error(400, "Expected a JSON object")
            return
        if not request.get("stream", False):
            payload = json.dumps({
                "id": "native-smoke-title",
                "object": "chat.completion",
                "model": "native-smoke",
                "choices": [{"index": 0, "message": {
                    "role": "assistant", "content": "原生界面验收"
                }, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 12, "completion_tokens": 6, "total_tokens": 18},
            }).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
            return
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Connection", "close")
        self.end_headers()
        for start in range(0, len(REPLY), 24):
            payload = {
                "id": "native-smoke",
                "object": "chat.completion.chunk",
                "model": "native-smoke",
                "choices": [{"index": 0, "delta": {"content": REPLY[start:start + 24]},
                             "finish_reason": None}],
            }
            self.wfile.write(("data: " + json.dumps(payload, ensure_ascii=False) + "\n\n").encode())
            self.wfile.flush()
            time.sleep(0.01)
        self.wfile.write(("data: " + json.dumps({
            "id": "native-smoke",
            "object": "chat.completion.chunk",
            "model": "native-smoke",
            "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 178000, "completion_tokens": 600, "total_tokens": 178600,
                      "prompt_tokens_details": {"cached_tokens": 177804}},
        }) + "\n\ndata: [DONE]\n\n").encode())
        self.wfile.flush()
        print("Local model: streamed response complete", flush=True)


def prepare(root: Path):
    for directory in ["workspace", "config/husk", "data", "home"]:
        (root / directory).mkdir(parents=True, exist_ok=True)
    (root / "native-smoke-profile").touch()
    workspace = root / "workspace"
    (workspace / ".gitignore").write_text(".husk/\n", encoding="utf-8")
    (workspace / "README.md").write_text("# Native smoke workspace\n", encoding="utf-8")
    git = ["git", "-C", str(workspace)]
    subprocess.run(git + ["init", "-q", "-b", "main"], check=True)
    subprocess.run(git + ["add", "."], check=True)
    subprocess.run(git + ["-c", "user.name=Native Smoke", "-c", "user.email=smoke@example.invalid",
                          "commit", "-qm", "Initialize isolated fixture"], check=True)
    (workspace / "README.md").write_text("# Native smoke workspace\n\n原生界面验收。\n", encoding="utf-8")
    for name in ["src/app.rs", "src/chat.rs", "src/terminal.rs", "docs/notes.md"]:
        path = workspace / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("pub fn fixture() -> bool { true }\n" if path.suffix == ".rs" else "# 验收笔记\n",
                        encoding="utf-8")
    (root / "home/.bashrc").write_text(
        "PS1='\\[\\e[0;32m\\]workspace\\[\\e[0m\\] on main\\n❯ '\n",
        encoding="utf-8",
    )
    (root / "home/.bash_profile").write_text('. "$HOME/.bashrc"\n', encoding="utf-8")


def active_window_pid():
    result = subprocess.run(
        ["gdbus", "call", "--session", "--dest", "org.kde.KWin",
         "--object-path", "/KWin", "--method", "org.kde.KWin.queryWindowInfo"],
        check=True, capture_output=True, text=True, timeout=10,
    )
    match = re.search(r"'pid':\s*<(?:uint32\s+)?(\d+)>", result.stdout)
    if match is None:
        raise RuntimeError("KWin did not return the active window's process id")
    return int(match.group(1))


def capture_window(process, root: Path):
    time.sleep(10)
    if process.poll() is not None:
        raise RuntimeError("native window exited before screenshot; inspect window.log")
    name = f"husk-native-smoke-{process.pid}"
    script = root / "activate-window.js"
    script.write_text(
        f"for (const window of workspace.windowList()) {{\n"
        f"  if (window.pid === {process.pid}) {{\n"
        "    window.minimized = false;\n"
        "    workspace.activeWindow = window;\n"
        "    break;\n"
        "  }\n"
        "}\n",
        encoding="utf-8",
    )
    base = ["gdbus", "call", "--session", "--dest", "org.kde.KWin"]
    loaded = subprocess.run(
        base + ["--object-path", "/Scripting", "--method",
                "org.kde.kwin.Scripting.loadScript", str(script), name],
        check=True, capture_output=True, text=True, timeout=10,
    )
    match = re.search(r"\((\d+),?\)", loaded.stdout)
    if match is None:
        raise RuntimeError(f"KWin did not return a script id: {loaded.stdout}")
    try:
        script_id = match.group(1)
        candidates = [f"/Scripting/Script{script_id}", f"/{script_id}", f"/Scripting/{script_id}"]
        for path in candidates:
            result = subprocess.run(
                ["gdbus", "introspect", "--session", "--dest", "org.kde.KWin",
                 "--object-path", path],
                capture_output=True, text=True, timeout=10,
            )
            if "org.kde.kwin.Script" in result.stdout:
                subprocess.run(base + ["--object-path", path, "--method", "org.kde.kwin.Script.run"],
                               check=True, capture_output=True, timeout=10)
                break
        else:
            raise RuntimeError("KWin script object is unavailable")
        time.sleep(0.5)
        output = root / "native.png"
        if active_window_pid() != process.pid:
            raise RuntimeError("the native window is not active; no screenshot was taken")
        try:
            subprocess.run(["spectacle", "--activewindow", "--background", "--nonotify",
                            "--output", str(output)], check=True, timeout=30)
            if active_window_pid() != process.pid:
                raise RuntimeError("active window changed during capture")
        except BaseException:
            output.unlink(missing_ok=True)
            raise
        print(f"Native screenshot: {output}", flush=True)
    finally:
        subprocess.run(base + ["--object-path", "/Scripting", "--method",
                               "org.kde.kwin.Scripting.unloadScript", name],
                       capture_output=True, timeout=10)


def check_parity(process, root: Path):
    """Guard every native input against the active isolated X11 window."""
    from PIL import Image

    def xdo(*args):
        return subprocess.check_output(["xdotool", *map(str, args)], text=True).strip()

    deadline = time.monotonic() + 15
    window = ""
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("native window exited before parity checks")
        try:
            window = xdo("search", "--onlyvisible", "--pid", process.pid).splitlines()[-1]
            break
        except (subprocess.CalledProcessError, IndexError):
            time.sleep(0.1)
    if not window:
        raise RuntimeError("parity checks require the isolated X11 window")
    xdo("windowactivate", "--sync", window)
    time.sleep(2)

    def capture(name):
        output = root / f"parity-{name}.png"
        subprocess.run(["import", "-window", window, str(output)], check=True, timeout=15)
        return output

    def key(value):
        # --window uses XSendEvent, which does not update the XKB state GPUI
        # reads for modifiers. Use XTest only while this exact window is active;
        # abort instead of stealing focus back if the user changed windows.
        if xdo("getactivewindow") != window:
            raise RuntimeError("active window changed; refusing to send test keys")
        xdo("key", "--clearmodifiers", value)
        if xdo("getactivewindow") != window:
            raise RuntimeError("active window changed during native input")

    def sidebar_width(path):
        with Image.open(path).convert("RGB") as image:
            y = image.height - 70
            row = [image.getpixel((x, y)) for x in range(min(image.width, 500))]
            spans, start = [], None
            for x, color in enumerate(row + [None]):
                matches = isinstance(color, tuple) and len(color) >= 3 and max(
                    abs(color[i] - (243, 243, 245)[i]) for i in range(3)
                ) <= 2
                if matches and start is None:
                    start = x
                elif not matches and start is not None:
                    spans.append((x - start, x))
                    start = None
            return max(spans, default=(0, 0))[1]

    baseline = capture("chat")
    initial = sidebar_width(baseline)
    if initial < 150:
        raise AssertionError(f"could not locate the light sidebar: {initial}")
    with Image.open(baseline) as image:
        image_width = image.width
        height = image.height
        y = height - 70
        rgb_image = image.convert("RGB")
        inset = next(x for x in range(initial) if rgb_image.getpixel((x, y)) == (243, 243, 245))
    scale = (initial - inset) / 240
    geometry = dict(line.split("=", 1) for line in xdo("getwindowgeometry", "--shell", window).splitlines())
    ratio_x, ratio_y = int(geometry["WIDTH"]) / image_width, int(geometry["HEIGHT"]) / height
    (root / "parity-input.json").write_text(json.dumps({
        "window":window, "geometry":geometry, "capture":[image_width, height],
        "input_scale":[ratio_x, ratio_y],
    }, indent=2) + "\n", encoding="utf-8")
    print(f"Native input geometry: {geometry['WIDTH']}x{geometry['HEIGHT']}; capture: {image_width}x{height}", flush=True)

    def click(x, y):
        if xdo("getactivewindow") != window:
            raise RuntimeError("active window changed; refusing to click")
        xdo("mousemove", "--window", window, round(x * ratio_x), round(y * ratio_y), "click", 1)

    # A newly created PTY can hold focus. Ctrl+B is intentionally readline's
    # binding there, not a shell shortcut. Focus the composer first.
    click(initial + round(100 * scale), round(height * 0.56))
    def toggle_sidebar(open_now):
        if xdo("getactivewindow") != window:
            raise RuntimeError("active window changed; refusing to click")
        x = (initial if open_now else inset) + round(22 * scale)
        click(x, inset + round(18 * scale))

    toggle_sidebar(True)
    started, samples = time.monotonic(), []
    for index in range(5):
        path = capture(f"sidebar-close-{index}")
        samples.append({"elapsed_ms": round((time.monotonic() - started) * 1000),
                        "width": sidebar_width(path)})
        time.sleep(0.035)
    time.sleep(0.35)
    closed = sidebar_width(capture("sidebar-closed"))
    if closed >= 10:
        raise AssertionError(f"sidebar did not finish closing: {closed}")
    if not any(5 < sample["width"] < initial - 5 for sample in samples):
        raise AssertionError(f"no intermediate sidebar geometry: {samples}")
    toggle_sidebar(False)
    time.sleep(0.4)
    restored = sidebar_width(capture("sidebar-restored"))
    if abs(restored - initial) > 3:
        raise AssertionError(f"sidebar did not restore its width: {initial} -> {restored}")
    click(initial - round(16 * scale), inset + round(88 * scale))
    time.sleep(0.2)
    capture("session-menu")
    key("Escape")
    time.sleep(0.2)
    click(inset + round(16 * scale), height - inset - round(16 * scale))
    time.sleep(0.3)
    capture("settings")
    key("Escape")
    time.sleep(0.4)
    capture("settings-return")
    key("ctrl+shift+b")
    time.sleep(0.15)
    capture("dock-closing")
    time.sleep(0.25)
    capture("dock-closed")
    key("ctrl+shift+b")
    time.sleep(0.4)
    capture("dock-restored")
    (root / "parity-results.json").write_text(json.dumps({
        "backend": "X11",
        "sidebar_initial": initial,
        "sidebar_closed": closed,
        "sidebar_restored": restored,
        "sidebar_close_samples": samples,
        "note": "Native loopback fixture. Screenshots need visual inspection; this is not a pixel-parity certification.",
    }, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Native parity artifacts: {root}", flush=True)
    key("ctrl+q")
    process.wait(timeout=15)


def smoke_environment(root: Path):
    return dict(os.environ, XDG_CONFIG_HOME=str(root / "config"),
                XDG_DATA_HOME=str(root / "data"), HOME=str(root / "home"),
                SHELL=shutil.which("bash") or "/bin/sh", NO_PROXY="localhost,127.0.0.1",
                no_proxy="localhost,127.0.0.1", HUSK_NATIVE_SMOKE_KEY="local-smoke-key",
                HUSK_GPUI_DISABLE_DESKTOP_NOTIFICATIONS="1")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, default=Path("target/debug/husk-gpui"))
    parser.add_argument("--seed", type=Path, default=Path("target/debug/examples/seed_reference"))
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--screenshot", action="store_true", help="capture the isolated window on KDE")
    parser.add_argument("--parity", action="store_true", help="exercise native menus/settings and assert intermediate sidebar geometry (X11)")
    parser.add_argument("--reuse-profile", type=Path, help="reuse a marked isolated smoke profile without replaying the local model")
    options = parser.parse_args()
    binary, seed = options.binary.resolve(), options.seed.resolve()
    if (not options.reuse_profile and not seed.is_file()) or (not options.prepare_only and not binary.is_file()):
        parser.error("build husk-gpui and the seed_reference example first")
    if options.reuse_profile:
        root = options.reuse_profile.resolve()
        if not root.is_dir() or not (root / "native-smoke-profile").is_file():
            parser.error("--reuse-profile requires an existing marked isolated profile")
    else:
        root = Path(tempfile.mkdtemp(prefix="husk-gpui-smoke-"))
        prepare(root)
    server = ThreadingHTTPServer(("127.0.0.1", 0), LocalModel)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    config = {
        "active_provider": "native-smoke",
        "active_model": "native-smoke",
        "providers": {"native-smoke": {
            "type": "openai_completions",
            "base_url": f"http://127.0.0.1:{server.server_port}/v1",
            "api_key": "env:HUSK_NATIVE_SMOKE_KEY",
            "models": [{"id": "native-smoke", "name": "Native QA", "contextWindow": 1000000}],
        }},
    }
    config_file = root / "config/husk/config.json"
    config_file.write_text(json.dumps(config), encoding="utf-8")
    config_file.chmod(0o600)
    env = smoke_environment(root)
    if options.parity:
        for command in ("xdotool", "import"):
            if not shutil.which(command):
                parser.error(f"{command} is required for --parity")
        env.pop("WAYLAND_DISPLAY", None)
    print(f"Isolated profile: {root}", flush=True)
    process = None
    try:
        if not options.reuse_profile:
            subprocess.run([str(seed), str(root)], cwd=root / "workspace", env=env,
                           timeout=150, check=True)
        if options.prepare_only:
            return
        with (root / "window.log").open("w") as log:
            process = subprocess.Popen([str(binary)], cwd=root / "workspace",
                                       env=env, stdout=log, stderr=subprocess.STDOUT)
            print(f"Native window PID: {process.pid}\nWindow log: {root / 'window.log'}\n"
                  f"Application log: {root / 'data/husk/logs/husk-gpui.log'}", flush=True)
            if options.parity:
                check_parity(process, root)
            elif options.screenshot:
                capture_window(process, root)
            code = process.wait()
            if code != 0:
                raise SystemExit(code)
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    main()
