"""Actual archive/build-script and safe test outcome validation."""

import hashlib
import json
import re
from pathlib import Path

from contracts import SAFE_TESTS, callback_exports, callback_policy, cargo_receipt


def sha(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        while block := stream.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def selected_outdir(target, crate):
    candidates = []
    for profile in ["release", "debug"]:
        for out in (target / profile / "build").glob(crate + "-*/out"):
            marker = out / "SwiftBridgeCore.swift"
            if marker.is_file():
                candidates.append((marker.stat().st_mtime_ns, out.resolve()))
    if len(candidates) != 1:
        raise RuntimeError("fresh phase must have exactly one eligible bridge OUTDIR; never infer from newest")
    return candidates[0][1]


def binding_receipt(evidence, manifest, package_id, target, release):
    matches = []
    for directory in evidence.iterdir():
        receipt = json.loads((directory / "receipt.json").read_text())
        args = receipt["forwarded_argv"]
        if args and args[0] == ("build" if release else "check") and "--manifest-path" in args:
            actual = Path(receipt["cwd"], args[args.index("--manifest-path") + 1]).resolve()
            if actual == manifest.resolve():
                matches.append((directory, receipt))
    if len(matches) != 1:
        raise RuntimeError("expected one actual canonical binding Cargo invocation")
    directory, invocation = matches[0]
    args = invocation["forwarded_argv"]
    if invocation["exit_code"] != 0 or invocation["lock_before"] != invocation["lock_after"]:
        raise RuntimeError("Cargo failed or changed source lock")
    messages = [json.loads(line) for line in (directory / "stdout.log").read_text().splitlines()]
    warnings = [
        m for m in messages if m.get("reason") == "compiler-message" and m.get("message", {}).get("level") == "warning"
    ]
    verbose = (directory / "stderr.log").read_text()
    rustc_lines = [line for line in verbose.splitlines() if "Running `" in line and "rustc " in line]
    if warnings or not rustc_lines or any("dynamic_lookup" in line for line in rustc_lines):
        raise RuntimeError("missing actual compiler argv or warnings/global allowance in native build")
    if not any("fatal_warnings" in line for line in rustc_lines):
        raise RuntimeError("strict linker flags absent from actual rustc command")
    result = cargo_receipt(map(json.dumps, messages), package_id, release)
    out = Path(result["out_dir"])
    if out != selected_outdir(target, "xberg-swift"):
        raise RuntimeError("producer-selected OUTDIR differs from actual Cargo receipt")
    owned = [m for m in messages if m.get("reason") == "compiler-artifact" and m.get("package_id") == package_id]
    if release:
        archives = [m for m in owned if "staticlib" in m.get("target", {}).get("crate_types", [])]
        if len(archives) != 1 or archives[0].get("profile", {}).get("test") is not False or "--release" not in args:
            raise RuntimeError("not a unique non-test staticlib artifact")
        archive = Path(result["archives"][0])
        if archive != target / "release/libxberg_swift.a" or archive.stat().st_size <= 1024:
            raise RuntimeError("archive is not real expected release staticlib")
        with archive.open("rb") as stream:
            if stream.read(8) != b"!<arch>\n":
                raise RuntimeError("expected actual archive magic")
        result["archive_sha256"] = sha(archive)
    result["raw_inputs"] = {
        str(path): sha(path)
        for path in [
            out / "SwiftBridgeCore.swift",
            out / "SwiftBridgeCore.h",
            out / "xberg-swift/xberg-swift.swift",
            out / "xberg-swift/xberg-swift.h",
        ]
    }
    build_outputs = [p for p in out.parent.glob("output") if p.is_file()]
    if len(build_outputs) != 1:
        raise RuntimeError("missing actual emitted build-script directives")
    swift = (out / "xberg-swift/xberg-swift.swift").read_text()
    result["callback_count"] = callback_policy(swift, build_outputs[0].read_text())
    return result


# SwiftPM 6.3 `-v` echoes the manifest compile as `warning: '<package>': <path>/swift-frontend -frontend -c ...`
# (run 41259357507); only that exact echo is dropped. ~keep
SWIFTPM_MANIFEST_ECHO = re.compile(r"^warning: '[^']+': /\S*/swift-frontend -frontend -c ")


def strict_output_warnings(text):
    return [line for line in text.splitlines() if "warning:" in line and not SWIFTPM_MANIFEST_ECHO.match(line)]


def tests_passed(text):
    names = re.findall(r"Test Case '-\[[^\]\n]+ (test\w+)\]' passed", text)
    if len(names) != 2 or set(names) != SAFE_TESTS:
        raise RuntimeError("expected exactly two named safe XCTest successes")
    if not re.search(r"Executed 2 tests, with 0 failures", text):
        raise RuntimeError("missing nonzero successful XCTest summary")
    return names


def missing_callbacks(text, expected):
    if "Undefined symbols" not in text or "linker command failed" not in text:
        raise RuntimeError("negative failed before actual final link")
    block = text.split("Undefined symbols", 1)[1].split("ld: symbol", 1)[0]
    names = set(re.findall(r'^\s+"(_[^"\n]+)", referenced from:', block, re.MULTILINE))
    if not names or names != expected:
        raise RuntimeError("actual missing callback linker set differs from archive-derived expectation")
    if strict_output_warnings(text):
        raise RuntimeError("unexpected warning in missing-callback control")
    return sorted(names)
