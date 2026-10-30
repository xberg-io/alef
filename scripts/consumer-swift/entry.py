"""The one admitted session spans bootstrap, cache allocation, consumer stages and fixture copies."""

import hashlib
import json
import os
import sys
import tempfile
from pathlib import Path

from contracts import FLOOR, immutable_ref
from runtime import free_receipt
from session_guard import supervise


def main():
    if sys.version_info < (3, 11):  # noqa: UP036  # ~keep Runs under the runner's preinstalled python3, not the mise interpreter.
        raise RuntimeError("hosted preinstalled Python 3.11+ required for actual TOML configuration")
    root = Path(os.environ.get("GITHUB_WORKSPACE", str(Path(__file__).resolve().parents[2]))).resolve(strict=True)
    os.environ.setdefault("GITHUB_WORKSPACE", str(root))
    mode = sys.argv[1] if len(sys.argv) == 2 else "verify"
    if mode not in {"verify", "--controls"} or len(sys.argv) > 2:
        raise ValueError("expected verify or --controls mode")
    if mode != "--controls" and os.environ.get("GITHUB_ACTIONS") != "true":
        raise RuntimeError("consumer proof requires the admitted hosted workflow")
    if mode != "--controls":
        immutable_ref(os.environ["ALEF_REF"])
        immutable_ref(os.environ["XBERG_REF"])
    if "RUNNER_TEMP" not in os.environ:
        os.environ["RUNNER_TEMP"] = tempfile.mkdtemp(prefix="alef-consumer-local-")
    names = {"verify": "consumer-evidence", "--controls": "consumer-controls-evidence"}
    evidence_root = Path(os.environ["RUNNER_TEMP"]).resolve(strict=True) if mode == "--controls" else root
    evidence = evidence_root / names[mode]
    evidence.mkdir(exist_ok=False)
    if mode == "--controls":
        os.environ["SWIFT_GATE_CONTROL_EVIDENCE"] = str(evidence)
    guard = Path(__file__).with_name("session_guard.py")
    expected = "3fe4d1f97b27b903075dae5b2e2182dbf23364c5976c573802d00b3753999643"
    if hashlib.sha256(guard.read_bytes()).hexdigest() != expected:
        raise RuntimeError("reviewed whole-SID guard changed")
    paths = execution_paths(root)
    before = free_receipt(paths)
    cap = {"verify": 16 * 1024**3, "--controls": 128 * 1024**2}[mode]
    if min(row["free"] for row in before.values()) < FLOOR + cap:
        (evidence / "refusal.json").write_text(json.dumps(before, indent=2))
        return 86
    os.environ["SWIFT_GATE_GUARDED"] = "1"
    records = []

    def sample():
        dynamic = evidence / "monitor-paths.json"
        if dynamic.is_file():
            for item in json.loads(dynamic.read_text()):
                path = Path(item).resolve(strict=True)
                if path not in paths:
                    paths.append(path)
        current = free_receipt(paths)
        records.append(current)
        for path, row in current.items():
            if path not in before:
                if row["free"] < FLOOR + cap:
                    return 0
                before[path] = row
        if any(before[path]["free"] - row["free"] > cap for path, row in current.items()):
            return 0
        return min(row["free"] for row in current.values())

    with (evidence / "whole-session.jsonl").open("x") as log:

        def emit(row):
            log.write(json.dumps(row) + "\n")
            log.flush()

        command = [sys.executable, str(Path(__file__).with_name("driver.py"))]
        if mode != "verify":
            command.append(mode)
        wall = {"verify": 10200, "--controls": 60}[mode]
        code = supervise(command, root, emit, floor=FLOOR, wall=wall, sampler=sample, interval=0.2)
    (evidence / "volume-samples.json").write_text(json.dumps(records))
    (evidence / "terminal.json").write_text(
        json.dumps({"exit": code, "whole_volume_decline_cap": cap, "attribution": "shared allocation not attributed"})
        + "\n"
    )
    return code


def execution_paths(root):
    paths = [
        root,
        Path.home(),
        Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))),
        Path(os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup"))),
        Path(os.environ["RUNNER_TEMP"]),
        Path(tempfile.gettempdir()),
        Path(os.environ.get("HOMEBREW_CACHE", str(Path.home() / "Library/Caches/Homebrew"))),
    ]
    for path in paths:
        path.mkdir(parents=True, exist_ok=True)
    return paths


if __name__ == "__main__":
    raise SystemExit(main())
