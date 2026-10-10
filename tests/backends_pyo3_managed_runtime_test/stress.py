# ~keep This opt-in subprocess test driver is not an importable Python package.
# ruff: noqa: INP001
"""Opt-in, interleaved native-extension exit comparison (not a mandatory test)."""

import argparse
import concurrent.futures
import json
import os
import random
import statistics
import subprocess
import sys
from pathlib import Path

CHILD = """
import asyncio, atexit, os, sys, time
if {cpus!r} and hasattr(os, 'sched_setaffinity'):
    os.sched_setaffinity(0, {cpus!r})
_hook_start_ns = None
def _finish_hook_timing():
    elapsed = time.monotonic_ns() - _hook_start_ns
    sys.stdout.write("EXIT_HOOK_TIMING_NS=" + str(elapsed) + "\\n")
    sys.stdout.flush()
def _start_hook_timing():
    global _hook_start_ns
    _hook_start_ns = time.monotonic_ns()
# LIFO observers surround the hook registered by import; controls keep the same observers. ~keep
atexit.register(_finish_hook_timing)
import _sample
if {clear_hook!r}:
    atexit._clear()
    atexit.register(_finish_hook_timing)
atexit.register(_start_hook_timing)
async def _main() -> None:
    for _ in range(20):
        assert await _sample.fetch() == 'done'
asyncio.run(_main())
sys.stdout.write("ASYNC_WORK_COMPLETED\\n")
sys.stdout.flush()
"""


def _run_case(case: tuple[str, Path, bool, list[int]]) -> tuple[str, int, bool, int | None]:
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
    markers = [
        line.removeprefix(b"EXIT_HOOK_TIMING_NS=")
        for line in result.stdout.splitlines()
        if line.startswith(b"EXIT_HOOK_TIMING_NS=")
    ]
    if len(markers) > 1:
        raise RuntimeError(f"{label} reported multiple exit-hook timings")
    elapsed_ns = int(markers[0]) if markers else None
    return label, result.returncode, completed, elapsed_ns


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
    durations = {label: [] for label, *_ in arms}
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
        for label, code, completed, elapsed_ns in pool.map(_run_case, cases):
            counts[label]["processes"] += 1
            counts[label]["completed_work"] += int(completed)
            counts[label]["nonzero"] += int(code != 0)
            counts[label]["signals"] += int(code < 0)
            if elapsed_ns is not None:
                durations[label].append(elapsed_ns)
    if not all(arm["processes"] == args.runs for arm in counts.values()):
        raise RuntimeError(f"unexpected process counts: {counts}")
    if len(durations["fixed"]) != args.runs:
        raise RuntimeError(f"expected {args.runs} fixed exit-hook timings, received {len(durations['fixed'])}")
    summaries = {
        label: {
            **counts[label],
            "exit_hook_samples": len(values),
            "exit_hook_median_ms": statistics.median(values) / 1_000_000 if values else None,
            "exit_hook_max_ms": max(values) / 1_000_000 if values else None,
        }
        for label, values in durations.items()
    }
    sys.stdout.write(json.dumps({"platform": sys.platform, "cpus": cpus, "results": summaries}, sort_keys=True) + "\n")


if __name__ == "__main__":
    _main()
