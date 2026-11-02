"""Actual native build provenance, not source-cache presence or system-link substitution."""

import json
from pathlib import Path

from receipts import sha


def package_name(package_id):
    # Cargo drops the name from a path package id when the directory matches it:
    # `path+file:///w/crates/xberg-tesseract#1.4.0`, but `registry+...#ort-sys@2.0.0`. ~keep
    source, fragment = package_id.rsplit("#", 1)
    if "@" in fragment:
        return fragment.split("@", 1)[0]
    return source.rstrip("/").rsplit("/", 1)[-1]


def static_archives(row):
    # ort-sys links its prebuilt from the ort.pyke.io download cache, which only `linked_paths` names. ~keep
    found = set(Path(row["out_dir"]).resolve(strict=True).rglob("*.a"))
    for entry in row.get("linked_paths") or []:
        kind, _, directory = entry.partition("=")
        path = Path(directory or kind)
        if (not directory or kind in ("native", "all")) and path.is_dir():
            found.update(path.resolve().glob("*.a"))
    return sorted(found)


def collect(invocations):
    scripts = []
    for directory in invocations.iterdir():
        for line in (directory / "stdout.log").read_text().splitlines():
            message = json.loads(line)
            if message.get("reason") == "build-script-executed":
                scripts.append(message)
    result = []
    for package in ["xberg-tesseract", "ort-sys"]:
        found = [row for row in scripts if package_name(row["package_id"]) == package]
        if len(found) != 1:
            raise RuntimeError("missing unique actual native build-script receipt: " + package)
        row = found[0]
        out = Path(row["out_dir"]).resolve(strict=True)
        archives = static_archives(row)
        if not archives or len(archives) > 300:
            raise RuntimeError("native static archive provenance missing or unexpectedly broad: " + package)
        native = {str(path.resolve()): {"bytes": path.stat().st_size, "sha256": sha(path)} for path in archives}
        result.append(
            {
                "package_id": row["package_id"],
                "out_dir": str(out),
                "archives": native,
                "linked_libs": row.get("linked_libs"),
                "linked_paths": row.get("linked_paths"),
                "cfgs": row.get("cfgs"),
                "env": row.get("env"),
            }
        )
    tesseract_names = {
        Path(path).name
        for row in result
        if package_name(row["package_id"]) == "xberg-tesseract"
        for path in row["archives"]
    }
    if not {"libtesseract.a", "libleptonica.a"}.issubset(tesseract_names):
        raise RuntimeError("both actual bundled OCR archives required")
    ort = [row for row in result if package_name(row["package_id"]) == "ort-sys"]
    ort_names = {Path(path).name for row in ort for path in row["archives"]}
    if "libonnxruntime.a" not in ort_names or not any(
        "static=onnxruntime" in (row["linked_libs"] or []) for row in ort
    ):
        raise RuntimeError("actual bundled static ORT archive required; system/dynamic replacement rejected")
    return result
