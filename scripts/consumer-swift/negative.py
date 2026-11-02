"""Actual generated-package negatives: remove exports, retain Swift function bodies."""

import re
import shutil
from pathlib import Path

from receipts import missing_callbacks, sha


def undefined_symbols(text):
    names = set()
    for line in text.splitlines():
        value = line.strip()
        if not value:
            continue
        if re.fullmatch(r"[^:\n]+\.(?:o|obj):|[^:\n]+\.a\([^()\n]+\.(?:o|obj)\):", value):
            continue
        valid = (
            re.fullmatch(r"_[A-Za-z0-9_$]+", value)
            or re.fullmatch(r"section\$(?:start|end)\$[A-Za-z0-9_]+\$[A-Za-z0-9_]+", value)
            or re.fullmatch(r"_objc_msgSend\$(?:[A-Za-z_][A-Za-z0-9_]*|(?:[A-Za-z_][A-Za-z0-9_]*:)+)", value)
        )
        if not valid:
            raise ValueError("unrecognized Darwin nm -u row; expected bare symbol or archive member header")
        names.add(value)
    if not names:
        raise ValueError("Darwin nm -u examined zero undefined symbols")
    return names


def strip_exports(root):
    names = set()
    files = []
    for path in root.rglob("*.swift"):
        source = path.read_text()
        found = re.findall(r'@_cdecl\("([^"\n]+)"\)', source)
        if not found:
            continue
        if names.intersection(found) or len(found) != len(set(found)):
            raise RuntimeError("duplicate generated callback exports")
        names.update(found)
        path.write_text(re.sub(r'@_cdecl\("([^"\n]+)"\)', "", source))
        files.append(str(path))
    if not names:
        raise RuntimeError("zero callback attributes removed")
    return names, files


def qualify(gate, archive, package):
    before = sha(archive)
    undefined = gate.command(
        "archive-undefined", [gate.real_nm, "--undefined-only", "--just-symbol-name", str(archive)], gate.xberg, 0, 90
    )
    referenced = undefined_symbols((undefined / "stdout.log").read_text())
    negative = gate.xberg / "packages/swift-missing-callbacks"
    if negative.exists():
        raise RuntimeError("negative package destination already exists")
    shutil.copytree(package, negative, ignore=shutil.ignore_patterns(".build", ".swiftpm"))
    names, changed = strip_exports(negative / "Sources")
    expected = referenced.intersection("_" + name for name in names)
    if not expected:
        raise RuntimeError("actual archive has zero references to removed generated callbacks")
    result = gate.command(
        "missing-callback-link",
        [gate.real_swift, "test", "-v", "-Xswiftc", "-warnings-as-errors", "-Xlinker", "-fatal_warnings"],
        negative,
        2 * 1024**3,
        600,
        expected=1,
    )
    combined = (result / "stdout.log").read_text() + (result / "stderr.log").read_text()
    observed = missing_callbacks(combined, expected)
    result = gate.command(
        "fatal-warning-link",
        [
            gate.real_swift,
            "test",
            "-v",
            "-Xswiftc",
            "-warnings-as-errors",
            "-Xlinker",
            "-fatal_warnings",
            "-Xlinker",
            "-lSystem",
        ],
        package,
        512 * 1024**2,
        300,
        expected=1,
    )
    diagnostic = (result / "stdout.log").read_text() + (result / "stderr.log").read_text()
    warnings = [line for line in diagnostic.splitlines() if "warning:" in line]
    if len(warnings) != 1 or "ignoring duplicate libraries" not in warnings[0] or "System" not in warnings[0]:
        raise RuntimeError("fatal-warning control failed for an unexpected reason")
    if "fatal" not in diagnostic or "linker command failed" not in diagnostic or "Undefined symbols" in diagnostic:
        raise RuntimeError("fatal-warning linker failure not proved")
    if sha(archive) != before:
        raise RuntimeError("negative controls changed actual produced archive")
    return {
        "removed_exports": len(names),
        "changed_negative_files": changed,
        "missing_archive_references": observed,
        "archive_unchanged": before,
    }
