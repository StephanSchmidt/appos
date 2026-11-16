"""Exercise the real CLI, Cargo and Wasm server: make check-dev (Unix)."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

from harness import CLI as cli, REPO as repo, available_port, logs_on_failure, request, stop, wait_ready


def main():
    with tempfile.TemporaryDirectory(prefix="appos-dev-regression-") as directory:
        root = Path(directory)
        env = {
            **os.environ,
            "APPOS_CONFIG": str(root / "config.toml"),
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_CONFIG_NOSYSTEM": "1",
        }
        env.pop("CARGO_TARGET_DIR", None)
        created = subprocess.run(
            [str(cli), "new", "events"], cwd=root, env=env, capture_output=True, text=True
        )
        assert created.returncode == 0, created.stderr
        app = root / "events"
        cache = repo / "cli/template/target"
        cache.mkdir(parents=True, exist_ok=True)
        (app / "target").symlink_to(cache, target_is_directory=True)

        # Edit after Cargo has built, before appos sees the build finish.
        # This deterministically exposes a watcher started too late.
        real_cargo = shutil.which("cargo")
        assert real_cargo, "cargo must be on PATH"
        tools = root / "bin"
        tools.mkdir()
        wrapper = tools / "cargo"
        wrapper.write_text(
            "#!" + sys.executable + "\n"
            "import pathlib, subprocess, sys\n"
            f"result = subprocess.run([{real_cargo!r}, *sys.argv[1:]])\n"
            f"marker = pathlib.Path({str(root / 'edited')!r})\n"
            "if result.returncode == 0 and sys.argv[1:2] == ['build'] and not marker.exists():\n"
            "    marker.touch()\n"
            "    p = pathlib.Path('templates/hello/index.html')\n"
            "    p.write_text(p.read_text().replace('Your new app', 'Changed app!'))\n"
            "sys.exit(result.returncode)\n"
        )
        wrapper.chmod(0o755)
        env["PATH"] = str(tools) + os.pathsep + env["PATH"]
        port = available_port()

        def serves(text):
            status, body = request(port, "GET", "/events")
            return status == 200 and text in body.decode("utf-8")

        with (root / "dev.log").open("w+") as log, logs_on_failure(dev=log):
            server = subprocess.Popen(
                [str(cli), "dev", "--addr", f"127.0.0.1:{port}"],
                cwd=app, env=env, stdout=log, stderr=log, start_new_session=True,
            )
            try:
                def wait_for(text, seconds):
                    wait_ready(server, lambda: serves(text), seconds, f"dev server never served {text!r}")

                wait_for("", 120)
                wait_for("Changed app!", 15)
                another = subprocess.run(
                    [str(cli), "dev", "--addr", f"127.0.0.1:{port}"],
                    cwd=app, env=env, capture_output=True, text=True, timeout=5,
                )
                assert another.returncode == 0 and "already running for this app" in another.stdout

                template = app / "templates/hello/index.html"
                original = template.stat()
                template.write_text(template.read_text().replace("Changed app!", "Last update!"))
                os.utime(template, ns=(original.st_atime_ns, original.st_mtime_ns))
                wait_for("Last update!", 15)
                print(json.dumps({
                    "edit_during_initial_build": "reloaded",
                    "preserved_mtime_edit": "reloaded",
                    "second_dev_server": "excluded",
                }))
            finally:
                # appos dev runs cargo: stop its whole process group.
                stop(server, group=True)


if __name__ == "__main__":
    main()
