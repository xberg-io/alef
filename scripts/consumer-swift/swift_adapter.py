"""Capture strict SwiftPM build/test output while preserving the canonical package test invocation."""

import os
import subprocess
import sys


def arguments(args, map_path):
    if not args or args[0] != "test":
        return list(args)
    if any(arg in args for arg in ["--skip-build", "--filter", "--skip"]):
        raise ValueError("canonical safe package must discover and execute all exactly two tests")
    return [
        *args,
        "-v",
        "-Xswiftc",
        "-warnings-as-errors",
        "-Xlinker",
        "-fatal_warnings",
        "-Xlinker",
        "-map",
        "-Xlinker",
        map_path,
    ]


if __name__ == "__main__":
    forwarded = arguments(sys.argv[1:], os.environ["SWIFT_GATE_LINK_MAP"])
    raise SystemExit(subprocess.call([os.environ["SWIFT_GATE_REAL_SWIFT"], *forwarded]))
