"""Hosted tool installation, all subprocesses admitted and monitored by the same runner."""

import json
import os
import re
import tomllib
from pathlib import Path

from receipts import sha

# Generation formatters, each installed from the mise config that pins it: the consumer's poly, and
# cargo-sort from the candidate, as xberg's mise.toml pins no cargo-sort. ~keep
MISE_TOOLS = [
    ("xberg", "github:Goldziher/poly", "poly"),
    ("alef", "github:DevinR528/cargo-sort", "cargo-sort"),
]


def bootstrap(gate):
    gate.env["RUSTUP_AUTO_INSTALL"] = "0"
    gate.command(
        "rust-toolchain",
        [
            "rustup",
            "toolchain",
            "install",
            "1.99.1",
            "--profile",
            "minimal",
            "--component",
            "rust-src",
            "--component",
            "rustfmt",
            "--component",
            "clippy",
            "--target",
            "wasm32-unknown-unknown",
            "--no-self-update",
        ],
        gate.alef,
        2 * 1024**3,
        600,
    )
    gate.command("rustup-identity", ["rustup", "--version"], gate.alef, 0, 10)
    configure_rust_native_environment(gate)
    install_mise_tools(gate)
    for program, version in [("rustc", "1.99.1"), ("cargo", "1.99.1")]:
        output = gate.command("version-" + program, [program, "--version"], gate.alef, 0, 15)
        if not reports_version((output / "stdout.log").read_text(), version):
            raise RuntimeError("installed tool version differs: " + program)
    cache = gate.command("actual-homebrew-cache", ["brew", "--cache"], gate.xberg, 0, 15)
    actual_cache = Path((cache / "stdout.log").read_text().strip())
    actual_cache.mkdir(parents=True, exist_ok=True)
    actual_cache = actual_cache.resolve(strict=True)
    gate.paths.append(actual_cache)
    gate.publish_paths()
    gate.command(
        "native-dependencies",
        ["bash", str(gate.xberg / "scripts/ci/install-system-deps/install-macos.sh")],
        gate.xberg,
        4 * 1024**3,
        1200,
    )
    install_clang_format(gate)
    for tool in ["cmake", "pkg-config", "swift", "clang", "clang-format"]:
        if not gate.which(tool):
            raise RuntimeError("strict generation/native tool unavailable: " + tool)


def reports_version(text, version):
    """True when `text` names `version` as a whole version, so 2.1.4 does not match 12.1.45."""
    return re.search(r"(?<![0-9.])" + re.escape(version) + r"(?![0-9.])", text) is not None


def configure_rust_native_environment(gate):
    sysroot = gate.command(
        "qualified-rust-sysroot", ["rustup", "run", "1.99.1", "rustc", "--print", "sysroot"], gate.alef, 0, 15
    )
    library = Path((sysroot / "stdout.log").read_text().strip()) / "lib"
    if not library.is_dir() or gate.env.get("DYLD_LIBRARY_PATH"):
        raise RuntimeError("matching Rust library missing or overriding loader environment requires review")
    configure_macos_deployment(gate)
    ambient = gate.env.get("DYLD_FALLBACK_LIBRARY_PATH", "")
    gate.env["DYLD_FALLBACK_LIBRARY_PATH"] = str(library) + (os.pathsep + ambient if ambient else "")


def install_mise_tools(gate):
    home = Path(gate.env["RUNNER_TEMP"]).resolve(strict=True) / "consumer-mise"
    home.mkdir()
    data = home / "data"
    gate.env.update(
        {
            "MISE_DATA_DIR": str(data),
            "MISE_CACHE_DIR": str(home / "cache"),
            "MISE_TRUSTED_CONFIG_PATHS": os.pathsep.join([str(gate.alef), str(gate.xberg)]),
            "MISE_LOCKED": "1",
            "MISE_YES": "1",
        }
    )
    mise = gate.which("mise")
    if not mise:
        raise RuntimeError("mise executable unavailable; the workflow installs it before the guarded session")
    resolved = Path(mise).resolve(strict=True)
    if any(resolved.is_relative_to(Path(root).resolve(strict=True)) for root in (gate.alef, gate.xberg)):
        raise RuntimeError("mise resolves inside a checkout under qualification: " + str(resolved))
    identity = gate.command("mise-identity", ["mise", "--version"], gate.alef, 0, 15)
    mise_version = (identity / "stdout.log").read_text().strip()
    if re.match(r"[0-9]{4}\.[0-9]+\.[0-9]+ ", mise_version + " ") is None:
        raise RuntimeError("mise --version does not report a mise release: " + mise_version)
    receipt = {
        "mise": str(resolved),
        "mise_sha256": sha(resolved),
        "mise_version": mise_version,
        "tools": [],
    }
    for checkout, tool, program in MISE_TOOLS:
        root = getattr(gate, checkout)
        config, lock = root / "mise.toml", root / "mise.lock"
        declared = tomllib.loads(config.read_text()).get("tools", {}).get(tool)
        if (
            not isinstance(declared, str)
            or re.fullmatch(r"[0-9]+(?:\.[0-9]+)*", declared) is None
            or not lock.is_file()
        ):
            raise RuntimeError("mise config must pin and lock generation tool: " + tool)
        gate.command(
            "install-" + program, ["mise", "install", "--locked", tool + "@" + declared], root, 512 * 1024**2, 600
        )
        selection = gate.command("mise-which-" + program, ["mise", "which", program], root, 0, 15)
        binary = Path((selection / "stdout.log").read_text().strip()).resolve(strict=True)
        if not binary.is_file() or not binary.is_relative_to(data.resolve(strict=True)):
            raise RuntimeError("mise selected an executable outside its private data directory: " + program)
        gate.env["PATH"] = str(binary.parent) + os.pathsep + gate.env["PATH"]
        if Path(gate.which(program) or "").resolve() != binary:
            raise RuntimeError("mise-installed executable is not selected on the generation PATH: " + program)
        version = gate.command("version-" + program, [binary, "--version"], gate.alef, 0, 15)
        if not reports_version((version / "stdout.log").read_text(), declared):
            raise RuntimeError("installed tool version differs: " + program)
        receipt["tools"].append(
            {
                "tool": tool,
                "version": declared,
                "config": str(config),
                "config_sha256": sha(config),
                "lock_sha256": sha(lock),
                "selected": str(binary),
                "executable_sha256": sha(binary),
            }
        )
    (gate.evidence / "mise-tools.json").write_text(json.dumps(receipt, indent=2) + "\n")


def install_clang_format(gate):
    formatter = gate.tools / "clang-format"
    gate.command("clang-format-venv", [gate.python, "-m", "venv", str(formatter)], gate.alef, 256 * 1024**2, 120)
    binary = formatter / "bin/clang-format"
    report = gate.evidence / "clang-format-install.json"
    gate.command(
        "install-clang-format",
        [
            str(formatter / "bin/python"),
            "-m",
            "pip",
            "--isolated",
            "install",
            "--only-binary=:all:",
            "--index-url",
            "https://pypi.org/simple",
            "--disable-pip-version-check",
            "--no-cache-dir",
            "--report",
            str(report),
            "clang-format==22.1.8",
        ],
        gate.alef,
        512 * 1024**2,
        600,
    )
    gate.env["PATH"] = str(binary.parent) + os.pathsep + gate.env["PATH"]
    selected = gate.which("clang-format")
    if not binary.is_file() or Path(selected or "").resolve() != binary.resolve():
        raise RuntimeError("pinned clang-format executable is not selected on the generation PATH")
    version = gate.command("version-clang-format", [str(binary), "--version"], gate.alef, 0, 15)
    if (version / "stdout.log").read_text().strip() != "clang-format version 22.1.8":
        raise RuntimeError("installed clang-format version differs")
    installed = json.loads(report.read_text())["install"]
    if (
        len(installed) != 1
        or installed[0]["metadata"]["name"] != "clang-format"
        or installed[0]["metadata"]["version"] != "22.1.8"
    ):
        raise RuntimeError("unexpected clang-format package installation report")
    download = installed[0]["download_info"]
    digest = download["archive_info"]["hashes"].get("sha256", "")
    if (
        not download["url"].startswith("https://files.pythonhosted.org/")
        or not download["url"].endswith(".whl")
        or re.fullmatch(r"[0-9a-f]{64}", digest) is None
    ):
        raise RuntimeError("official clang-format wheel provenance missing")
    (gate.evidence / "clang-format-tool.json").write_text(
        json.dumps(
            {
                "selected": str(binary.resolve()),
                "executable_sha256": sha(binary),
                "version": "22.1.8",
                "wheel": download,
            },
            indent=2,
        )
        + "\n"
    )


def swift_package_macos_floor(xberg):
    """The shared `.macOS(...)` floor, with two components and a nonzero patch when present."""
    floors = set()
    for manifest in [xberg / "packages/swift/Package.swift", xberg / "Package.swift"]:
        if not manifest.is_file():
            continue
        found = re.findall(r'\.macOS\(\s*(?:\.v(\d+)(?:_(\d+))?|"(\d+(?:\.\d+){0,2})")\s*\)', manifest.read_text())
        if len(found) != 1:
            raise RuntimeError("Swift manifest must declare exactly one macOS platform floor: " + manifest.name)
        major, minor, text = found[0]
        floors.add(normalized_version(text or major + "." + (minor or "0")))
    if len(floors) != 1:
        raise RuntimeError("shipped Swift manifests must declare one shared macOS platform floor")
    major, minor, patch = floors.pop()
    return f"{major}.{minor}" + (f".{patch}" if patch else "")


def normalized_version(text):
    parts = tuple(map(int, text.split(".")))
    return parts + (0,) * (3 - len(parts))


def configure_macos_deployment(gate):
    if gate.env.get("CARGO_BUILD_TARGET"):
        raise RuntimeError("explicit Cargo build target requires deployment-target review")
    require_host_cargo_configuration(gate)
    # Rust, cc and the prebuilt objects it links (ONNX Runtime's are built for macOS 13.4) must all
    # target the floor the Swift package ships with; rustc's own 11.0 default qualified a deployment
    # target no consumer gets, and ld64 reports the newer objects under -fatal_warnings. ~keep
    floor = swift_package_macos_floor(gate.xberg)
    requested = gate.env.get("MACOSX_DEPLOYMENT_TARGET")
    if requested is not None:
        if re.fullmatch(r"[0-9]+(?:\.[0-9]+){0,2}", requested) is None:
            raise RuntimeError("explicit macOS deployment target requires review")
        if normalized_version(requested) != normalized_version(floor):
            raise RuntimeError("explicit macOS deployment target differs from the Swift package floor")
    else:
        gate.env["MACOSX_DEPLOYMENT_TARGET"] = floor
    stage = gate.command(
        "qualified-macos-deployment-target",
        ["rustup", "run", "1.99.1", "rustc", "--print=deployment-target"],
        gate.alef,
        0,
        15,
    )
    value = (stage / "stdout.log").read_text().strip()
    match = re.fullmatch(r"MACOSX_DEPLOYMENT_TARGET=([0-9]+\.[0-9]+(?:\.[0-9]+)?)", value)
    if match is None or (stage / "stderr.log").read_text():
        raise RuntimeError("qualified Rust macOS deployment target is invalid or has diagnostics")
    selected = match.group(1)
    if normalized_version(selected) != normalized_version(floor):
        raise RuntimeError("qualified Rust macOS deployment target differs from the Swift package floor")
    (gate.evidence / "macos-deployment-target.json").write_text(
        json.dumps(
            {
                "requested": requested,
                "swift_package_floor": floor,
                "rust_selected": selected,
                "effective": gate.env["MACOSX_DEPLOYMENT_TARGET"],
            },
            indent=2,
        )
        + "\n"
    )


def require_host_cargo_configuration(gate):
    configured_home = gate.env.get("CARGO_HOME")
    home = Path(configured_home) if configured_home is not None else Path.home() / ".cargo"
    if configured_home == "" or not home.is_absolute():
        raise RuntimeError("relative or empty Cargo home requires deployment-target review")
    configurations = {home / "config", home / "config.toml"}
    for checkout in [gate.alef, gate.xberg]:
        for directory in [checkout, *checkout.parents]:
            configurations.update([directory / ".cargo/config", directory / ".cargo/config.toml"])
    receipts = []
    for path in sorted(configurations):
        if not path.exists() and not path.is_symlink():
            continue
        try:
            document = tomllib.loads(path.read_text())
        except (OSError, UnicodeError, tomllib.TOMLDecodeError) as error:
            raise RuntimeError(
                "unreadable or malformed Cargo configuration requires deployment-target review"
            ) from error
        build = document.get("build", {})
        if not isinstance(build, dict) or "target" in build:
            raise RuntimeError("Cargo configuration build target requires deployment-target review")
        configured_environment = document.get("env", {})
        if not isinstance(configured_environment, dict) or any(
            key in configured_environment for key in ["MACOSX_DEPLOYMENT_TARGET", "CARGO_BUILD_TARGET"]
        ):
            raise RuntimeError("Cargo configuration deployment environment requires review")
        receipts.append({"path": str(path), "sha256": sha(path)})
    (gate.evidence / "cargo-host-configuration.json").write_text(json.dumps(receipts, indent=2) + "\n")
