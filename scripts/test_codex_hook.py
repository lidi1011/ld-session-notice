"""Must-block tests for the Codex hook script (src-tauri/src/hooks/codex-hook.sh).

Runs the real script with /bin/sh against a fake Notice server on a free port,
with a temporary token. No real Notice, token or Codex is touched.

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
CASES = ROOT / "src-tauri/src/rules/risk_cases.tsv"
OLD_RULE = ("rm -rf /", "sudo rm", "diskutil erase", "drop table", "truncate table")

ANSWERS = {
    "/allow": (200, {"continue": True, "suppressOutput": True}),
    "/deny": (200, {"continue": False, "stopReason": "Rejected in Notice"}),
    "/failopen": (200, {"continue": True, "stopReason": "Notice failed open: db"}),
    "/unauthorized": (401, {"continue": False, "stopReason": "Notice token rejected"}),
    "/error": (500, {"continue": True}),
}


def load_cases():
    cases = []
    for line in CASES.read_text().splitlines():
        if line and not line.startswith("#"):
            expected, command = line.split("\t", 1)
            cases.append((expected == "critical", command))
    return cases


class FakeNotice(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers.get("Content-Length", 0)))
        if self.path == "/slow":
            time.sleep(31)
            return
        code, body = ANSWERS[self.path]
        data = json.dumps(body).encode()
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
        cls.cases = load_cases()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.tmp.cleanup()

    def run_hook(self, command, url, event="PreToolUse", legacy=0, token=None):
        script = (
            TEMPLATE.read_text()
            .replace("__TOKEN_FILE__", f"'{token or self.token}'")
            .replace("__URL__", f"'{url}'")
            .replace("__LEGACY_SERVER__", str(legacy))
        )
        path = Path(self.tmp.name) / f"hook-{legacy}.sh"
        path.write_text(script)
        payload = {
            "hook_event_name": event,
            "tool_name": "Bash",
            "tool_input": {"command": command},
            "cwd": "/Users/someone/project",
        }
        return subprocess.run(
            ["/bin/sh", str(path)],
            input=json.dumps(payload, ensure_ascii=False),
            capture_output=True,
            text=True,
            timeout=40,
        )

    def assert_blocked(self, result, command):
        self.assertEqual(result.returncode, 2, f"{command!r} was not blocked")
        self.assertIn("Notice blocked this command", result.stderr)
        self.assertEqual(result.stdout, "")

    def assert_allowed(self, result, command):
        self.assertEqual(result.returncode, 0, f"{command!r}: {result.stderr}")
        self.assertEqual(result.stdout, "", "Codex rejects continue/suppressOutput on PreToolUse")

    def test_notice_down_blocks_only_destructive(self):
        for critical, command in self.cases:
            result = self.run_hook(command, self.down)
            if critical:
                self.assert_blocked(result, command)
            else:
                self.assert_allowed(result, command)

    def test_unicode_space_variant_blocked(self):
        for command in ("rm -rf /", "rm　-fr /"):
            self.assert_blocked(self.run_hook(command, self.down), command)

    def test_current_server_decides(self):
        for _, command in self.cases:
            self.assert_allowed(self.run_hook(command, self.base + "/allow"), command)

    def test_legacy_server_cannot_approve_new_spellings(self):
        for critical, command in self.cases:
            result = self.run_hook(command, self.base + "/allow", legacy=1)
            old_rule_hit = any(rule in " ".join(command.lower().split()) for rule in OLD_RULE)
            if critical and not old_rule_hit:
                self.assert_blocked(result, command)
            else:
                self.assert_allowed(result, command)

    def test_rejection_blocks(self):
        result = self.run_hook("rm -rf /", self.base + "/deny")
        self.assert_blocked(result, "rm -rf /")
        self.assertIn("Rejected in Notice", result.stderr)

    def test_other_events_never_block_or_print(self):
        for event in ("SessionStart", "PostToolUse", "PermissionRequest", "Stop"):
            result = self.run_hook("rm -rf /", self.base + "/deny", event=event)
            self.assertEqual((result.returncode, result.stdout), (0, ""), event)

    def test_bad_answers_block_destructive(self):
        for path in ("/unauthorized", "/error", "/failopen"):
            self.assert_blocked(self.run_hook("rm -fr /", self.base + path), path)
            self.assert_allowed(self.run_hook("ls -la /", self.base + path), path)

    def test_missing_token_blocks_destructive(self):
        missing = Path(self.tmp.name) / "no-token"
        self.assert_blocked(self.run_hook("sudo  rm -r /x", self.down, token=missing), "sudo rm")
        self.assert_allowed(self.run_hook("echo hi", self.down, token=missing), "echo hi")

    def test_timeout_blocks(self):
        started = time.monotonic()
        self.assert_blocked(self.run_hook("echo hi", self.base + "/slow"), "slow answer")
        self.assertLess(time.monotonic() - started, 30, "must finish inside Codex's 30 s hook timeout")


if __name__ == "__main__":
    unittest.main()
