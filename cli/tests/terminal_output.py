"""Native console regressions: make check-console (no Wasm target needed)."""

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import zipfile

from harness import CLI, REPO, SHELL, available_port, request, stop, wait_ready, wait_until


CONTROL = "\x1b[2J"
MESSAGE = "before" + CONTROL + "\rhidden\n[other] forged"


# A fake platform. Its answers are the shell's api types (shell/src/api.rs)
# as JSON; the CLI reads them strictly, so a missing field fails this test.
class Handler(BaseHTTPRequestHandler):
    fail_versions = False

    def do_GET(self):
        status = 200
        if self.path.endswith("/versions"):
            if self.fail_versions:
                status, payload = 422, "refused" + CONTROL
            else:
                payload = [{"id": 1, "app_id": "test", "label": "1-" + CONTROL, "version": "1",
                            "name": "Test", "owner": "", "group": "", "status": "live",
                            "reason": "why" + CONTROL, "deployed_at": 0, "took_ms": 0,
                            "grants": [], "routes": []}]
        elif self.path.endswith("/news"):
            payload = {"now": 1, "events": [], "more": 0, "paused": None, "off": None}
        elif self.path.startswith("/_platform/log?"):
            payload = {"events": [{
                "id": 1, "at": 0, "app": "test", "label": "1-" + CONTROL, "kind": "limit-hit",
                "severity": "high", "actor": "shell", "message": "message" + CONTROL,
                "reason": "why" + CONTROL, "risk": None, "recovery_ms": None,
                "failed": {"count": 1, "first": [{"at": 0, "label": "1-" + CONTROL,
                          "route": "GET /" + CONTROL, "outcome": "crashed", "wall_us": 0,
                          "error": "error" + CONTROL}]},
            }], "older": None}
        else:
            payload = {"name": "platform" + CONTROL, "origin": "http://apps.example" + CONTROL,
                       "framework": "appos:framework@0.1.0", "capabilities": [], "risk": {},
                       "apps": [], "git": None,
                       "wit": {"appos.wit": "// " + CONTROL + "\npackage appos:framework@0.1.0;"}}
        body = payload.encode() if isinstance(payload, str) else json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


def component(crash=False):
    # Keep the normal deployment fixture unchanged: the short-hash collision
    # regression relies on its exact contents. Add logging only to this copy.
    source = (REPO / "shell/src/testing.wat").read_text()
    source = source.replace("  (type $pairs", '''  (import "appos:framework/log@0.1.0" (instance $log
    (export "info" (func (param "message" string)))))
  (alias export $log "info" (func $info))
  (type $pairs''', 1)
    source = source.replace("  (core module $app", '''  (core func $info (canon lower (func $info) (memory $memory)))
  (core module $app''', 1)
    data = "".join("\\%02x" % b for b in MESSAGE.encode())
    source = source.replace("    (data (i32.const 0)",
        '    (import "host" "info" (func $info (param i32 i32)))\n'
        f'    (data (i32.const 128) "{data}")\n    (data (i32.const 0)', 1)
    source = source.replace('    (func (export "start") (param i32)',
        '    (func (export "start") (param i32)\n'
        f'      i32.const 128 i32.const {len(MESSAGE.encode())} call $info', 1)
    source = source.replace('(export "html" (func $html))))))',
        '(export "html" (func $html)) (export "info" (func $info))))))', 1)
    if crash:
        source = source.replace("i32.const 2 call $html)", "i32.const 2 call $html unreachable)", 1)
    return source


def main():
    with tempfile.TemporaryDirectory(prefix="appos-console-regression-") as tmp:
        root = Path(tmp)
        manifest = "[app]\nid='test'\nname='Test'\nversion='1'\nowner='Tests'\ngroup='Tests'\n"
        (root / "app.toml").write_text(manifest)
        (root / "app.wasm").write_text(component())
        env = {**os.environ, "APPOS_CONFIG": str(root / "config.toml")}
        api = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=api.serve_forever, daemon=True)
        thread.start()
        commands = [
            ["login", f"http://127.0.0.1:{api.server_port}"],
            ["platforms"], ["hosts"], ["hosts", "--wit"], ["versions"], ["log"],
        ]
        try:
            for args in commands:
                result = subprocess.run([str(CLI), *args],
                    cwd=root, env=env, capture_output=True, timeout=10)
                assert result.returncode == 0, (args, repr(result.stderr))
                assert b"\x1b" not in result.stdout + result.stderr, (args, "raw terminal control")
            Handler.fail_versions = True
            result = subprocess.run([str(CLI), "versions"],
                cwd=root, env=env, capture_output=True, timeout=10)
            assert result.returncode != 0 and b"\\u{1b}[2J" in result.stderr
            assert b"\x1b" not in result.stderr
            result = subprocess.run([str(SHELL), "--risk", "missing" + CONTROL],
                cwd=root, capture_output=True, timeout=10)
            assert result.returncode != 0 and b"\\u{1b}[2J" in result.stderr
            assert b"\x1b" not in result.stderr
        finally:
            api.shutdown()
            api.server_close()
            thread.join(timeout=5)

        outcomes = {}
        for mode in ["normal", "stderr", "stdout", "failed-request", "stalled-stderr"]:
            archive = io.BytesIO()
            with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as bundle:
                bundle.writestr("app.toml", manifest)
                bundle.writestr("app.wasm", component(crash=mode == "failed-request"))
            port = available_port()
            close_stderr = mode in ["stderr", "failed-request"]
            stalled_reader = None
            stalled_writer = None
            if mode == "stalled-stderr":
                read_fd, stalled_writer = os.pipe()
                stalled_reader = os.fdopen(read_fd, "rb")
                # Fill an owned pipe without depending on OS pipe capacity.
                os.set_blocking(stalled_writer, False)
                try:
                    while True:
                        os.write(stalled_writer, b"x" * 4096)
                except BlockingIOError:
                    pass
                os.set_blocking(stalled_writer, True)
            with (root / f"stdout-{mode}").open("wb") as out, (root / f"stderr-{mode}").open("w+b") as err:
                server = subprocess.Popen([
                    str(SHELL), "--no-auth", "--addr", f"127.0.0.1:{port}",
                    "--data", str(root / f"data-{mode}"), str(root / "app.toml"),
                ], stdout=subprocess.PIPE if mode == "stdout" else out,
                   stderr=stalled_writer if stalled_reader else subprocess.PIPE if close_stderr else err,
                   env={**os.environ, "TOKIO_WORKER_THREADS": "1"})
                if stalled_writer is not None:
                    os.close(stalled_writer)
                try:
                    wait_ready(server, lambda: request(port, "GET", "/")[0] == 200, 10, "shell did not start")
                    status, body = request(port, "POST", "/_platform/deploy",
                                           archive.getvalue(), {"Content-Type": "application/zip"})
                    assert status == 200, (mode, status, repr(body))
                    if close_stderr:
                        server.stderr.close()
                    if mode == "stdout":
                        server.stdout.close()
                    status, _ = request(port, "GET", "/test")
                    assert status == (503 if mode == "failed-request" else 200), (mode, status)
                    if not close_stderr and not stalled_reader:
                        # The logging worker runs independently of the response.
                        # Read through a separate descriptor to avoid moving its offset.
                        log = root / f"stderr-{mode}"
                        wait_until(lambda: b"\n[test] [other] forged\n" in log.read_bytes(), 2, lambda: repr(log.read_bytes()))
                        logs = log.read_bytes()
                        assert b"\x1b" not in logs and b"\r" not in logs, repr(logs)
                    if stalled_reader:
                        # More than the bounded queue can hold: saturation must
                        # drop console messages while the only async worker serves.
                        for _ in range(80):
                            assert request(port, "GET", "/test")[0] == 200
                        assert request(port, "GET", "/")[0] == 200
                    if mode == "stdout":
                        status, body = request(port, "POST", "/_platform/apps/test/off", "{}", {"Content-Type": "application/json"})
                        assert status == 200, (status, repr(body))
                        assert request(port, "GET", "/test")[0] == 503
                    outcomes[mode] = "passed"
                finally:
                    if stalled_reader:
                        stalled_reader.close()
                    stop(server)
        print(json.dumps({"cli_commands": len(commands), "cli_error_response": "escaped", "native_logs": outcomes}))


if __name__ == "__main__":
    main()
