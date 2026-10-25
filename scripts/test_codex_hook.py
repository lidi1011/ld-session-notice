"""Tests for the Codex hook script (src-tauri/src/hooks/codex-hook.sh).

Notice only records Codex activity; approving commands is left to Codex. So
the hook must forward every event and never block, print or stall Codex,
whatever state Notice is in. Runs the real script with /bin/sh against a fake
Notice server on a free port, with a temporary token.

    python3 scripts/test_codex_hook.py
"""

import json
import socket
import subprocess
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TEMPLATE = ROOT / "src-tauri/src/hooks/codex-hook.sh"
COMMANDS = ("rm -rf /", "rm -fr /", "sudo  rm -r /x", "echo hi")
EVENTS = ("SessionStart", "PreToolUse", "PostToolUse", "PermissionRequest", "Stop")

ANSWERS = {
    "/allow": (200, {"continue": True, "suppressOutput": True}),
    "/deny": (200, {"continue": False, "stopReason": "Rejected in Notice"}),
    "/unauthorized": (401, {"continue": False, "stopReason": "Notice token rejected"}),
    "/error": (500, {"continue": True}),
}


class FakeNotice(BaseHTTPRequestHandler):
    received = []

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        FakeNotice.received.append((self.headers.get("X-Notice-Token"), json.loads(body)))
        if self.path == "/slow":
            time.sleep(10)
            return
        code, answer = ANSWERS[self.path]
        data = json.dumps(answer).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


class CodexHookTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.token = Path(cls.tmp.name) / "token"
        cls.token.write_text("test-token")
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), FakeNotice)
        cls.server.daemon_threads = True
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        cls.base = f"http://127.0.0.1:{cls.server.server_address[1]}"
        cls.down = f"http://127.0.0.1:{free_port()}/api/webhook/codex"

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.tmp.cleanup()

    def run_hook(self, command, url, event="PreToolUse", token=None):
        script = (
            TEMPLATE.read_text()
            .replace("__TOKEN_FILE__", f"'{token or self.token}'")
            .replace("__URL__", f"'{url}'")
        )
        path = Path(self.tmp.name) / "hook.sh"
        path.write_text(script)
        payload = {
            "hook_event_name": event,
            "tool_name": "Bash",
            "tool_input": {"command": command},
        }
        started = time.monotonic()
        result = subprocess.run(
            ["/bin/sh", str(path)],
            input=json.dumps(payload, ensure_ascii=False),
            capture_output=True,
            text=True,
            timeout=20,
        )
        return result, time.monotonic() - started

    def assert_passes(self, result, label):
        self.assertEqual(result.returncode, 0, f"{label}: {result.stderr}")
        self.assertEqual(result.stdout, "", f"{label}: Codex rejects JSON fields on PreToolUse")

    def test_never_blocks_whatever_notice_answers(self):
        for url in [self.down] + [self.base + path for path in ANSWERS]:
            for event in EVENTS:
                for command in COMMANDS:
                    result, _ = self.run_hook(command, url, event=event)
                    self.assert_passes(result, f"{url} {event} {command}")

    def test_forwards_payload_with_token(self):
        FakeNotice.received.clear()
        self.run_hook("rm -fr /", self.base + "/allow")
        token, payload = FakeNotice.received[-1]
        self.assertEqual(token, "test-token")
        self.assertEqual(payload["tool_input"]["command"], "rm -fr /")

    def test_missing_token_passes_without_calling(self):
        FakeNotice.received.clear()
        result, _ = self.run_hook("rm -rf /", self.base + "/allow", token=Path(self.tmp.name) / "none")
        self.assert_passes(result, "missing token")
        self.assertEqual(FakeNotice.received, [])

    def test_slow_notice_does_not_stall_codex(self):
        result, took = self.run_hook("echo hi", self.base + "/slow")
        self.assert_passes(result, "slow")
        self.assertLess(took, 5)


if __name__ == "__main__":
    unittest.main()
