import importlib.util
import io
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch


SPEC = importlib.util.spec_from_file_location(
    "native_smoke", Path(__file__).resolve().parents[1] / "native-smoke.py"
)
assert SPEC is not None and SPEC.loader is not None
smoke = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(smoke)


class ModelRequestTests(unittest.TestCase):
    def test_invalid_requests_return_bad_request(self):
        for body, length in [
            (b"{broken", "7"),
            (b"[]", "2"),
            (b"{}", "-1"),
            (b"{}", "invalid"),
            (b'"\xff"', "3"),
        ]:
            with self.subTest(body=body, length=length):
                handler = SimpleNamespace(
                    headers={"Content-Length": length},
                    rfile=io.BytesIO(body),
                    send_error=Mock(),
                )
                smoke.LocalModel.respond(handler)
                handler.send_error.assert_called_once()
                self.assertEqual(handler.send_error.call_args.args[0], 400)


class ScreenshotTests(unittest.TestCase):
    def test_isolated_profile_always_disables_system_notifications(self):
        root = Path(tempfile.gettempdir()) / "smoke-profile"
        env = smoke.smoke_environment(root)
        self.assertEqual(env["HUSK_GPUI_DISABLE_DESKTOP_NOTIFICATIONS"], "1")
        self.assertEqual(env["XDG_CONFIG_HOME"], str(root / "config"))
        self.assertEqual(env["XDG_DATA_HOME"], str(root / "data"))

    def test_active_window_pid_requires_a_pid(self):
        with patch.object(smoke.subprocess, "run") as run:
            run.return_value.stdout = "({'pid': <uint32 123>},)"
            self.assertEqual(smoke.active_window_pid(), 123)
            run.return_value.stdout = "({'caption': <'other'>},)"
            with self.assertRaises(RuntimeError):
                smoke.active_window_pid()

    def test_focus_change_discards_the_image(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            process = SimpleNamespace(pid=123, poll=lambda: None)
            calls = []

            def run(args, **_kwargs):
                calls.append(args)
                if "org.kde.kwin.Scripting.loadScript" in args:
                    return SimpleNamespace(stdout="(42,)")
                if "introspect" in args:
                    return SimpleNamespace(stdout="org.kde.kwin.Script")
                if args[0] == "spectacle":
                    (root / "native.png").write_bytes(b"not the target")
                return SimpleNamespace(stdout="")

            with (
                patch.object(smoke.time, "sleep"),
                patch.object(smoke.subprocess, "run", side_effect=run),
                patch.object(smoke, "active_window_pid", side_effect=[123, 456]),
            ):
                with self.assertRaisesRegex(RuntimeError, "changed during capture"):
                    smoke.capture_window(process, root)
            self.assertFalse((root / "native.png").exists())
            self.assertTrue(any("org.kde.kwin.Scripting.unloadScript" in call for call in calls))

    def test_wrong_active_window_does_not_start_capture(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            process = SimpleNamespace(pid=123, poll=lambda: None)
            calls = []

            def run(args, **_kwargs):
                calls.append(args)
                if "org.kde.kwin.Scripting.loadScript" in args:
                    return SimpleNamespace(stdout="(42,)")
                if "introspect" in args:
                    return SimpleNamespace(stdout="org.kde.kwin.Script")
                return SimpleNamespace(stdout="")

            with (
                patch.object(smoke.time, "sleep"),
                patch.object(smoke.subprocess, "run", side_effect=run),
                patch.object(smoke, "active_window_pid", return_value=456),
            ):
                with self.assertRaisesRegex(RuntimeError, "no screenshot was taken"):
                    smoke.capture_window(process, root)
            self.assertFalse(any(call[0] == "spectacle" for call in calls))


if __name__ == "__main__":
    unittest.main()
