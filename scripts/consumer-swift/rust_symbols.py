"""Read Mach-O and Rust bitcode with the LLVM shipped by the selected Rust toolchain."""

import json
import os
import re
import tomllib
from pathlib import Path

from receipts import sha


def declaration_path():
    return Path(__file__).resolve().parents[2] / "rust-toolchain.toml"


def toolchain_channel():
    return tomllib.loads(declaration_path().read_text())["toolchain"]["channel"]


def install_arguments():
    declared = tomllib.loads(declaration_path().read_text())["toolchain"]
    if "llvm-tools" not in declared["components"]:
        raise RuntimeError("controller Rust toolchain must declare llvm-tools")
    args = ["rustup", "toolchain", "install", declared["channel"], "--profile", declared["profile"]]
    for option, values in [("--component", declared["components"]), ("--target", declared["targets"])]:
        for value in values:
            args.extend([option, value])
    return [*args, "--no-self-update"]


def compiler_identity(text, channel):
    identity = {}
    for key in ["release", "host", "LLVM version"]:
        values = re.findall(r"^" + re.escape(key) + r": (.+)$", text, re.MULTILINE)
        if len(values) != 1:
            raise RuntimeError("invalid selected Rust compiler identity")
        identity[key] = values[0]
    if (
        identity["release"] != channel
        or re.fullmatch(r"[a-z0-9_]+-apple-darwin", identity["host"]) is None
        or re.fullmatch(r"\d+\.\d+\.\d+", identity["LLVM version"]) is None
    ):
        raise RuntimeError("invalid selected Rust compiler identity")
    return identity


def matching_binary(sysroot, host):
    try:
        root = sysroot.resolve(strict=True)
        binary = (root / "lib/rustlib" / host / "bin/llvm-nm").resolve(strict=True)
        if binary.is_relative_to(root) and binary.is_file() and os.access(binary, os.X_OK):
            return binary
    except OSError as error:
        raise RuntimeError("matching Rust llvm-nm is unavailable") from error
    raise RuntimeError("matching Rust llvm-nm is unavailable")


def select(gate):
    declaration = declaration_path()
    channel = tomllib.loads(declaration.read_text())["toolchain"]["channel"]
    if gate.env.get("RUSTUP_TOOLCHAIN") != channel:
        raise RuntimeError("symbol reader toolchain differs from the selected Rust compiler")
    stage = gate.command("symbol-rustc-identity", ["rustup", "run", channel, "rustc", "-vV"], gate.alef, 0, 15)
    identity = compiler_identity((stage / "stdout.log").read_text(), channel)
    stage = gate.command(
        "symbol-rust-sysroot", ["rustup", "run", channel, "rustc", "--print", "sysroot"], gate.alef, 0, 15
    )
    binary = matching_binary(Path((stage / "stdout.log").read_text().strip()), identity["host"])
    stage = gate.command("symbol-nm-version", [binary, "--version"], gate.alef, 0, 15)
    versions = re.findall(r"LLVM version (\d+\.\d+\.\d+)", (stage / "stdout.log").read_text())
    if versions != [identity["LLVM version"]]:
        raise RuntimeError("LLVM reader differs from the selected Rust compiler")
    report = {
        "rust_release": identity["release"],
        "host": identity["host"],
        "llvm_version": identity["LLVM version"],
        "selected": str(binary),
        "executable_sha256": sha(binary),
        "declaration": str(declaration),
        "declaration_sha256": sha(declaration),
    }
    (gate.evidence / "rust-symbol-tool.json").write_text(json.dumps(report, indent=2) + "\n")
    gate.real_nm = str(binary)
