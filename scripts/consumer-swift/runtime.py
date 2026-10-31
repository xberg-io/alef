"""Bounded hosted-only commands; shared-volume floors include all actual cache paths."""

import json
import os
import shutil
import subprocess
import threading
import time
from contextlib import suppress
from pathlib import Path

from contracts import FLOOR

LOG_LIMIT = 64 * 1024**2


def session_allocation(current):
    """Bytes the enclosing session has spent so far against its whole-volume decline cap."""
    cap = int(os.environ["SWIFT_GATE_SESSION_CAP"])
    baseline = json.loads(os.environ["SWIFT_GATE_SESSION_BASELINE"])
    declines = [baseline[path]["free"] - row["free"] for path, row in current.items() if path in baseline]
    if cap <= 0 or not declines:
        raise ValueError("session cap and a shared monitored baseline path required")
    return {"cap": cap, "spent": max(declines)}


def free_receipt(paths):
    result = {}
    for path in paths:
        actual = Path(path).resolve(strict=True)
        info = actual.stat()
        result[str(actual)] = {"device": info.st_dev, "free": shutil.disk_usage(actual).free}
    if not result:
        raise ValueError("zero monitored filesystem paths")
    return result


def capture_stream(source, destination, child, errors):
    total = 0
    try:
        # read() waits for a full block, so a killed stage lost up to 64 KiB of its last output. ~keep
        while block := source.read1(65536):
            total += len(block)
            if total > LOG_LIMIT:
                errors.append("bounded stage log exceeded")
                child.kill()
                return
            destination.write(block)
            destination.flush()
    except (OSError, ValueError) as error:
        errors.append("stage stream capture failed: " + type(error).__name__)
        with suppress(ProcessLookupError):
            child.kill()


def run(name, argv, cwd, *, env, evidence, paths, budget, timeout, expected=0):
    if os.environ.get("SWIFT_GATE_GUARDED") != "1":
        raise RuntimeError("stage requires enclosing reviewed whole-SID supervisor")
    stage = evidence / name
    stage.mkdir()
    before = free_receipt(paths)
    if min(row["free"] for row in before.values()) < FLOOR + budget:
        (stage / "refusal.json").write_text(json.dumps(before, indent=2))
        raise RuntimeError("stage admission floor plus explicit allocation cap not met")
    # The session guard reports a crossed cap as zero free bytes and kills every stage unattributed,
    # so a stage whose own budget no longer fits under the cap is refused here, by name. ~keep
    session = session_allocation(before)
    if session["spent"] + budget > session["cap"]:
        (stage / "refusal.json").write_text(
            json.dumps({"before": before, "session": session, "budget": budget}, indent=2)
        )
        raise RuntimeError("stage allocation exceeds the remaining whole-session cap: " + name)
    start = time.monotonic()
    samples, errors = [], []
    with (stage / "stdout.log").open("xb") as stdout, (stage / "stderr.log").open("xb") as stderr:
        child = subprocess.Popen(argv, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        readers = [
            threading.Thread(target=capture_stream, args=(child.stdout, stdout, child, errors), daemon=True),
            threading.Thread(target=capture_stream, args=(child.stderr, stderr, child, errors), daemon=True),
        ]
        for thread in readers:
            thread.start()
        try:
            while child.poll() is None:
                current = free_receipt(paths)
                samples.append({"elapsed": time.monotonic() - start, "volumes": current})
                if min(row["free"] for row in current.values()) < FLOOR:
                    errors.append("live free-space floor crossed")
                if any(before[path]["free"] - row["free"] > budget for path, row in current.items()):
                    errors.append("shared-volume free-space decline exceeded stage cap; attribution unknown")
                if time.monotonic() - start > timeout:
                    errors.append("stage aggregate timeout")
                if errors:
                    child.terminate()
                    break
                time.sleep(0.2)
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=2)
        finally:
            for thread in readers:
                thread.join(timeout=5)
            if any(thread.is_alive() for thread in readers):
                errors.append("stream reader did not finish after descendant cleanup")
    after = free_receipt(paths)
    if min(row["free"] for row in after.values()) < FLOOR:
        errors.append("post-stage free-space floor crossed")
    receipt = {
        "argv": argv,
        "cwd": str(cwd),
        "exit": child.returncode,
        "expected": expected,
        "before": before,
        "after": after,
        "samples": samples,
        "errors": errors,
        "elapsed": time.monotonic() - start,
        "budget": budget,
        "cleanup_owner": "enclosing reviewed whole-SID supervisor",
    }
    (stage / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    if errors or child.returncode != expected:
        raise RuntimeError("stage failed: " + name)
    return stage
