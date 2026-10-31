"""Unexecuted forwarding adapter draft; outer reviewed whole-SID guard is mandatory."""

import hashlib
import json
import os
import subprocess
import sys
import threading
import time
from pathlib import Path

from contracts import cargo_arguments

LIMIT = int(os.environ.get("SWIFT_GATE_STREAM_LIMIT_BYTES", str(64 * 1024 * 1024)))
if not 0 < LIMIT <= 64 * 1024 * 1024:
    raise ValueError("invalid bounded Cargo stream allowance")


def relay(source, destination, log, child, failures):
    size = 0
    try:
        # read() waits for a full block, so a killed stage lost up to 64 KiB of its last output. ~keep
        while block := source.read1(65536):
            size += len(block)
            if size > LIMIT:
                raise ValueError("Cargo stream exceeded bounded evidence allowance")
            log.write(block)
            destination.write(block)
            destination.flush()
    except (OSError, ValueError) as error:
        failures.append(str(error))
        child.kill()


def main():
    real = Path(os.environ["SWIFT_GATE_REAL_CARGO"]).resolve(strict=True)
    if not real.is_file() or not os.access(real, os.X_OK):
        raise ValueError("actual executable Cargo required")
    evidence = Path(os.environ["SWIFT_GATE_CARGO_EVIDENCE"]).resolve(strict=True)
    arguments = cargo_arguments(sys.argv[1:])
    manifest = Path(os.environ["SWIFT_GATE_LOCK"]).resolve(strict=True)
    lock_before = hashlib.sha256(manifest.read_bytes()).hexdigest()
    # An exclusive directory records every Cargo invocation without guessing process ordering. ~keep
    invocation = evidence / (str(time.time_ns()) + "-" + str(os.getpid()))
    invocation.mkdir()
    failures = []
    with (invocation / "stdout.log").open("xb") as out, (invocation / "stderr.log").open("xb") as err:
        child = subprocess.Popen([str(real), *arguments], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        threads = [
            threading.Thread(target=relay, args=(child.stdout, sys.stdout.buffer, out, child, failures)),
            threading.Thread(target=relay, args=(child.stderr, sys.stderr.buffer, err, child, failures)),
        ]
        for thread in threads:
            thread.start()
        code = child.wait()
        for thread in threads:
            thread.join()
    lock_after = hashlib.sha256(manifest.read_bytes()).hexdigest()
    receipt = {
        "cargo": str(real),
        "cwd": str(Path.cwd()),
        "original_argv": sys.argv[1:],
        "forwarded_argv": arguments,
        "exit_code": code,
        "failures": failures,
        "lock_before": lock_before,
        "lock_after": lock_after,
    }
    (invocation / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    if failures or lock_before != lock_after:
        return 97
    return code if code >= 0 else 128 + abs(code)


if __name__ == "__main__":
    raise SystemExit(main())
