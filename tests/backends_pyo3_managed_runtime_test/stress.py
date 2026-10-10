# ~keep This opt-in subprocess test driver is not an importable Python package.
# ruff: noqa: INP001
"""Opt-in, interleaved native-extension exit comparison (not a mandatory test)."""

import argparse
import concurrent.futures
import json
import os
import random
import subprocess
import sys
from pathlib import Path

CHILD = """
import asyncio, atexit, os, sys
if {cpus!r} and hasattr(os, 'sched_setaffinity'):
    os.sched_setaffinity(0, {cpus!r})
import _sample
if {clear_hook!r}:
    atexit._clear()
async def _main() -> None:
    for _ in range(20):
        assert await _sample.fetch() == 'done'
asyncio.run(_main())
sys.stdout.write("ASYNC_WORK_COMPLETED\\n")
sys.stdout.flush()
"""


def _run_case(case: tuple[str, Path, bool, list[int]]) -> tuple[str, int, bool]:
    label, directory, clear_hook, cpus = case
    result = subprocess.run(
        [sys.executable, "-c", CHILD.format(cpus=cpus, clear_hook=clear_hook)],
        env={**os.environ, "PYTHONPATH": str(directory)},
        capture_output=True,
        timeout=15,
        check=False,
    )
    completed = b"ASYNC_WORK_COMPLETED" in result.stdout
    if result.returncode > 0 and not completed:
        raise RuntimeError(f"{label} workload failed before finalization: {result.stderr.decode(errors='replace')}")
    return label, result.returncode, completed


def _main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--baseline", type=Path, required=True, help="directory with preserved baseline _sample extension"
    )
    parser.add_argument("--fixed", type=Path, required=True, help="directory with preserved fixed _sample extension")
    parser.add_argument("--runs", type=int, default=2000, help="processes per comparison arm")
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--cpus", default="", help="Linux affinity CPUs, for example 0,1")
    args = parser.parse_args()
    if args.runs < 1 or args.workers < 1:
        parser.error("runs and workers must be positive")
    for directory in [args.baseline, args.fixed]:
        if not directory.is_dir() or not list(directory.glob("_sample*.so")):
            parser.error(f"missing preserved extension: {directory}")
    cpus = [int(cpu) for cpu in args.cpus.split(",") if cpu]
    arms = [
        ("baseline", args.baseline, False, cpus),
        ("fixed", args.fixed, False, cpus),
        ("hook_cleared", args.fixed, True, cpus),
    ]
    cases = arms * args.runs
    random.Random(525).shuffle(cases)  # noqa: S311 — reproducible scheduling, no security use. ~keep
    counts = {label: {"processes": 0, "completed_work": 0, "nonzero": 0, "signals": 0} for label, *_ in arms}
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
        for label, code, completed in pool.map(_run_case, cases):
            counts[label]["processes"] += 1
            counts[label]["completed_work"] += int(completed)
            counts[label]["nonzero"] += int(code != 0)
            counts[label]["signals"] += int(code < 0)
    if not all(arm["processes"] == args.runs for arm in counts.values()):
        raise RuntimeError(f"unexpected process counts: {counts}")
    sys.stdout.write(json.dumps({"platform": sys.platform, "cpus": cpus, "results": counts}, sort_keys=True) + "\n")


if __name__ == "__main__":
    _main()
