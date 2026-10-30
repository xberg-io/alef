"""Exercise Alef's actual pnpm plans without public registry or package installation."""

# ruff: noqa: INP001 -- standalone subprocess fixture, not an importable package

import http.server
import json
import os
import pathlib
import shlex
import subprocess
import sys
import tempfile
import threading

PACKAGE = "alef-workspace-update-probe"
MANIFESTS = ("package.json", "packages/child/package.json")


class _Registry(http.server.BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        """Serve only local package metadata; tarball downloads fail."""
        if self.path != f"/{PACKAGE}":
            self.send_error(404)
            return
        versions = {
            version: {
                "name": PACKAGE,
                "version": version,
                "dist": {
                    "tarball": f"http://127.0.0.1:{self.server.server_port}/{version}.tgz",
                    "integrity": "sha512-" + "A" * 86 + "==",
                },
            }
            for version in ("1.0.0", "2.0.0")
        }
        body = json.dumps({"name": PACKAGE, "dist-tags": {"latest": "2.0.0"}, "versions": versions}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args: object) -> None:
        """Keep HTTP access chatter out of the counted test receipt."""


def _require(condition: bool, message: object) -> None:
    if not condition:
        raise AssertionError(message)


def _check_plan(pnpm: str, commands: list[str], registry: str) -> int:
    _require(commands[0] == "pnpm self-update", commands)
    updates = [shlex.split(command) for command in commands[1:]]
    _require(bool(updates), commands)
    _require(all(args[:2] == ["pnpm", "up"] for args in updates), commands)
    with tempfile.TemporaryDirectory(prefix="alef-pnpm-update-") as directory:
        root = pathlib.Path(directory)
        (root / "packages/child").mkdir(parents=True)
        for relative, name in zip(MANIFESTS, ("root", "child"), strict=True):
            manifest = {"name": name, "private": True, "devDependencies": {PACKAGE: "1.0.0"}}
            (root / relative).write_text(json.dumps(manifest), encoding="utf-8")
        (root / "pnpm-workspace.yaml").write_text("packages:\n  - 'packages/*'\n", encoding="utf-8")
        for args in updates:
            completed = subprocess.run(
                [pnpm, *args[1:], "--lockfile-only", "--ignore-scripts", f"--registry={registry}"],
                cwd=root,
                capture_output=True,
                text=True,
                timeout=90,
                env={**os.environ, "CI": "true"},
                check=False,
            )
            _require(completed.returncode == 0, (completed.stdout, completed.stderr))
        versions = [
            json.loads((root / relative).read_text(encoding="utf-8"))["devDependencies"][PACKAGE]
            for relative in MANIFESTS
        ]
        sys.stdout.write(f"manifests={len(versions)} root_child={versions}\n")
        sys.stdout.flush()
        _require(versions == ["2.0.0", "2.0.0"], "both workspace manifests must advance")
        return len(versions)


def _main() -> None:
    plans = json.load(sys.stdin)
    _require(len(plans) == 2, "Node and WASM plans must both run")
    with http.server.ThreadingHTTPServer(("127.0.0.1", 0), _Registry) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            registry = f"http://127.0.0.1:{server.server_port}"
            manifests = sum(_check_plan(sys.argv[1], commands, registry) for commands in plans)
            sys.stdout.write(f"plans={len(plans)} manifests={manifests}\n")
        finally:
            server.shutdown()
            thread.join()


if __name__ == "__main__":
    _main()
