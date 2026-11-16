"""What the cli/tests share: the binaries, free ports, HTTP requests, waits
with a deadline, processes that are always stopped, and logs shown on failure. Every wait has a timeout."""

import contextlib
import http.client
import os
from pathlib import Path
import signal
import socket
import subprocess
import time


REPO = Path(__file__).resolve().parents[2]
CLI = REPO / "target/debug/appos"
SHELL = REPO / "target/debug/appos-shell"


def available_port():
    """A port free right now on 127.0.0.1."""
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        return reservation.getsockname()[1]


def request(port, method, path, body=None, headers=None, timeout=5):
    """One HTTP request to 127.0.0.1: (status, body bytes)."""
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
    try:
        connection.request(method, path, body, headers or {})
        response = connection.getresponse()
        return response.status, response.read()
    finally:
        connection.close()


def wait_until(check, seconds, what):
    """Calls check() until it returns something true and returns that. An
    OSError (nothing listening yet) counts as not yet. After `seconds`, fails
    with `what` (a text, or a function giving one)."""
    deadline = time.monotonic() + seconds
    while True:
        try:
            result = check()
            if result:
                return result
        except OSError:
            pass
        if time.monotonic() >= deadline:
            raise AssertionError(f"timed out after {seconds} s: {what() if callable(what) else what}")
        time.sleep(0.05)


def wait_ready(process, check, seconds, what):
    """wait_until, failing at once if `process` exits meanwhile."""
    def probe():
        assert process.poll() is None, f"{what}: the process exited"
        return check()
    return wait_until(probe, seconds, what)


def stop(process, group=False, seconds=10):
    """Stops a process this test started (with its process group if it got
    one), killing it if it doesn't end in time."""
    if process is None or process.poll() is not None:
        return
    send = (lambda s: os.killpg(process.pid, s)) if group else process.send_signal
    send(signal.SIGTERM)
    try:
        process.wait(timeout=seconds)
    except subprocess.TimeoutExpired:
        send(signal.SIGKILL)
        process.wait(timeout=5)


@contextlib.contextmanager
def logs_on_failure(**logs):
    """Prints the named log files (open w+) if the block fails."""
    try:
        yield
    except BaseException:
        for label, log in logs.items():
            log.flush()
            log.seek(0)
            print(label, log.read())
        raise


def start_shell(log, *args, ready_port, ready_path, seconds=10):
    """Starts appos-shell with `args`, logging to `log`; returns it once
    GET ready_path on ready_port answers 200."""
    server = subprocess.Popen([str(SHELL), *args], stdout=log, stderr=log)
    try:
        wait_ready(server, lambda: request(ready_port, "GET", ready_path, timeout=2)[0] == 200, seconds, "the shell never became ready")
    except BaseException:
        stop(server)
        raise
    return server
