"""Runnable hosted consumer prototype. Any unsupported/missing proof aborts the gate."""

import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path

import bootstrap
import materialized
import native_receipts
import negative
import receipts
import runtime
from contracts import immutable_ref, safe_test_census
from strict_flags import strict_flags


class Gate:
    def __init__(self):
        self.root = Path(os.environ["GITHUB_WORKSPACE"]).resolve(strict=True)
        self.source = Path(__file__).resolve().parent
        self.python = sys.executable
        self.evidence = self.root / "consumer-evidence"
        self.alef, self.xberg = self.root / "alef", self.root / "xberg"
        self.tools = self.evidence / "tools"
        self.bin = self.tools / "bin"
        self.bin.mkdir(parents=True)
        self.env = dict(os.environ)
        self.env.update(
            {
                "RUSTUP_TOOLCHAIN": "1.99.1",
                "CARGO_BUILD_JOBS": "1",
                "CARGO_INCREMENTAL": "0",
                "REPO_ROOT": str(self.xberg),
                "PATH": str(self.bin) + os.pathsep + self.env["PATH"],
            }
        )
        self.paths = [
            self.root,
            Path.home(),
            Path(self.env.get("CARGO_HOME", str(Path.home() / ".cargo"))),
            Path(self.env.get("RUSTUP_HOME", str(Path.home() / ".rustup"))),
            Path(self.env["RUNNER_TEMP"]),
            Path(tempfile.gettempdir()),
            Path(self.env.get("HOMEBREW_CACHE", str(Path.home() / "Library/Caches/Homebrew"))),
        ]
        self.real_cargo, self.real_swift = self.which("cargo"), self.which("swift")
        if not self.real_cargo or not self.real_swift or platform.system() != "Darwin":
            raise RuntimeError("actual hosted macOS Rustup/Cargo/Swift required")
        self.publish_paths()

    def publish_paths(self):
        for path in self.paths:
            path.mkdir(parents=True, exist_ok=True)
        source = self.evidence / "monitor-paths-next.json"
        source.write_text(json.dumps(list(map(str, self.paths))))
        source.replace(self.evidence / "monitor-paths.json")

    def which(self, command):
        return shutil.which(command, path=self.env["PATH"])

    def command(self, name, argv, cwd, budget, timeout, *, expected=0):
        return runtime.run(
            name,
            list(map(str, argv)),
            cwd,
            env=self.env,
            evidence=self.evidence,
            paths=self.paths,
            budget=max(budget, 64 * 1024**2),
            timeout=timeout,
            expected=expected,
        )

    def require_head(self, root, expected):
        expected = immutable_ref(expected)
        head = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
        dirty = subprocess.check_output(["git", "-C", str(root), "status", "--porcelain"], text=True)
        if head != expected or dirty:
            raise RuntimeError("checkout must equal requested immutable SHA and be clean")

    def checkout(self, name, repository, revision):
        revision = immutable_ref(revision)
        root = self.root / name
        if root.exists():
            raise RuntimeError("fresh immutable checkout path already exists: " + name)
        root.mkdir()
        for suffix, argv in [
            ("init", ["git", "init"]),
            ("origin", ["git", "remote", "add", "origin", "https://github.com/" + repository + ".git"]),
            ("fetch", ["git", "fetch", "--depth", "1", "origin", revision]),
            ("checkout", ["git", "checkout", "--detach", "FETCH_HEAD"]),
        ]:
            self.command(name + "-" + suffix, argv, root, 1024**3, 180)
        self.require_head(root, revision)

    def install_adapters(self):
        selected = self.command(
            "actual-cargo-selection", ["rustup", "which", "--toolchain", "1.99.1", "cargo"], self.alef, 0, 10
        )
        self.real_cargo = (selected / "stdout.log").read_text().strip()
        if not Path(self.real_cargo).is_file():
            raise RuntimeError("Rustup did not select an actual Cargo executable")
        self.env.update(
            {
                "SWIFT_GATE_REAL_CARGO": self.real_cargo,
                "SWIFT_GATE_REAL_SWIFT": self.real_swift,
                "SWIFT_GATE_LINK_MAP": str(self.evidence / "positive-link.map"),
            }
        )
        for tool, script in [("cargo", "cargo_adapter.py"), ("swift", "swift_adapter.py")]:
            path = self.bin / tool
            path.write_text(
                "#!"
                + sys.executable
                + "\nimport runpy, sys\nsys.path.insert(0, "
                + repr(str(self.source))
                + ")\nrunpy.run_path("
                + repr(str(self.source / script))
                + ", run_name='__main__')\n"
            )
            path.chmod(0o755)

    def phase(self, name, root, target):
        if target.exists():
            raise RuntimeError("phase target directory must not already exist")
        self.env["CARGO_TARGET_DIR"] = str(target)
        self.env["SWIFT_GATE_LOCK"] = str(root / "Cargo.lock")
        invocation = self.evidence / (name + "-cargo")
        invocation.mkdir()
        self.env["SWIFT_GATE_CARGO_EVIDENCE"] = str(invocation)
        return invocation

    def metadata(self, name, manifest, target):
        stage = self.command(
            name,
            ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1", "--manifest-path", str(manifest)],
            self.xberg,
            0,
            120,
        )
        metadata = json.loads((stage / "stdout.log").read_text())
        if Path(metadata["target_directory"]).resolve() != target.resolve():
            raise RuntimeError("actual Cargo target directory differs from phase target")
        packages = [p for p in metadata["packages"] if Path(p["manifest_path"]).resolve() == manifest.resolve()]
        if len(packages) != 1:
            raise RuntimeError("binding package missing from actual metadata")
        return packages[0]["id"]

    def build_cli(self):
        invocations = self.phase("alef", self.alef, self.alef / "target")
        self.command("build-current-alef", ["cargo", "build", "--release", "-p", "alef"], self.alef, 4 * 1024**3, 1800)
        candidates = []
        for directory in invocations.iterdir():
            for line in (directory / "stdout.log").read_text().splitlines():
                message = json.loads(line)
                if (
                    message.get("reason") == "compiler-artifact"
                    and message.get("target", {}).get("name") == "alef"
                    and message.get("executable")
                ):
                    candidates.append(Path(message["executable"]).resolve(strict=True))
        if len(candidates) != 1 or candidates[0] != self.alef / "target/release/alef":
            raise RuntimeError("missing unique actual candidate CLI Cargo artifact")
        cli = self.evidence / "alef-frozen"
        shutil.copy2(candidates[0], cli)
        stage = self.command("current-cli-version", [cli, "--version"], self.alef, 0, 15)
        version = (stage / "stdout.log").read_text()
        if os.environ["ALEF_REF"] not in version or "dirty" in version:
            raise RuntimeError("CLI does not report clean exact candidate full commit")
        self.require_head(self.alef, os.environ["ALEF_REF"])
        return cli

    def configure_strict_flags(self):
        extra = [
            Path.home() / ".cargo/config",
            Path.home() / ".cargo/config.toml",
            self.root / ".cargo/config",
            self.root / ".cargo/config.toml",
        ]
        if any(path.is_file() and "rustflags" in path.read_text() for path in extra):
            raise RuntimeError("additional Cargo rustflags source needs explicit merge review; preserve it")
        config = tomllib.loads((self.xberg / ".cargo/config.toml").read_text())
        configured = config["target"]['cfg(target_os = "macos")']["rustflags"]
        flags, removed = strict_flags(self.env, configured)
        (self.evidence / "strict-flags.json").write_text(
            json.dumps(
                {
                    "ambient_rustflags": self.env.get("RUSTFLAGS"),
                    "ambient_encoded": self.env.get("CARGO_ENCODED_RUSTFLAGS"),
                    "configured": configured,
                    "actual_encoded_items": flags,
                    "exact_allowances_removed": removed,
                },
                indent=2,
            )
        )
        self.env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(flags)

    def fetch_consumer_dependencies(self):
        # alef resolves the `from_registry` source crates in xberg's alef.toml with a full `cargo metadata`,
        # which on a cold runner downloads and unpacks the whole locked graph (over 1 GiB for 9b0a9387, runs
        # 41115788579 and 41125475169). Fetching it first keeps that download out of the extract allocation.
        self.command(
            "fetch-current-consumer-dependencies",
            ["cargo", "fetch", "--locked", "--manifest-path", self.xberg / "Cargo.toml"],
            self.xberg,
            4 * 1024**3,
            900,
        )

    def consumer(self, cli):
        package = self.xberg / "packages/swift"
        manifest = package / "rust/Cargo.toml"
        safe_test_census((package / "Tests/XbergTests/XbergTests.swift").read_text())
        lock = receipts.sha(self.xberg / "Cargo.lock")
        check_target = self.evidence / "check-target"
        check_calls = self.phase("check", self.xberg, check_target)
        self.fetch_consumer_dependencies()
        common = [cli, "--config", self.xberg / "alef.toml", "--no-color", "--jobs", "1", "--crate", "xberg"]
        self.command(
            "extract-current-consumer",
            [*common, "extract", "--output", self.evidence / "ir.json"],
            self.xberg,
            1024 * 1024**2,
            300,
        )
        ir = json.loads((self.evidence / "ir.json").read_text())
        census = {name: len(ir[name]) for name in ["types", "functions", "enums", "errors", "services"]}
        if any(census[name] == 0 for name in ["types", "functions", "enums"]):
            raise RuntimeError("empty configured consumer IR")
        self.command(
            "full-current-generation",
            [*common, "generate", "--lang", "swift", "--clean", "--strict"],
            self.xberg,
            8 * 1024**3,
            2400,
        )
        package_id = self.metadata("actual-check-metadata", manifest, check_target)
        check = receipts.binding_receipt(check_calls, manifest, package_id, check_target, False)
        materialized.verify(Path(check["out_dir"]), package)
        release_target = self.xberg / "target"
        release_calls = self.phase("release", self.xberg, release_target)
        self.command(
            "sanctioned-current-build",
            [*common, "build", "--lang", "swift", "--release", "--strict"],
            self.xberg,
            8 * 1024**3,
            3600,
        )
        package_id = self.metadata("actual-release-metadata", manifest, release_target)
        release = receipts.binding_receipt(release_calls, manifest, package_id, release_target, True)
        release["native_builds"] = native_receipts.collect(release_calls)
        materialized.verify(Path(release["out_dir"]), package)
        frozen = self.evidence / "frozen-release"
        frozen.mkdir()
        archive = Path(release["archives"][0])
        shutil.copy2(archive, frozen / archive.name)
        if receipts.sha(frozen / archive.name) != release["archive_sha256"]:
            raise RuntimeError("frozen actual archive copy differs")
        for source in release["raw_inputs"]:
            raw_source = Path(source)
            destination = frozen / raw_source.relative_to(Path(release["out_dir"]))
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(raw_source, destination)
        if receipts.sha(self.xberg / "Cargo.lock") != lock:
            raise RuntimeError("source Cargo.lock changed")
        # xberg's mise tasks define no Swift package test: `swift:e2e` runs the separate e2e package and
        # `test:bindings` every language. These are the steps of its former `swift:test` task, which alef.toml
        # `[crates.test.swift]` still declares as `cd packages/swift && swift test`. ~keep
        output = ""
        for name, argv in [
            (
                "canonical-swift-binding-build",
                ["cargo", "build", "--locked", "--release", "--manifest-path", "rust/Cargo.toml"],
            ),
            ("canonical-safe-package-tests", ["swift", "test"]),
        ]:
            stage = self.command(name, argv, package, 2 * 1024**3, 900)
            output += (stage / "stdout.log").read_text() + (stage / "stderr.log").read_text()
        tests = receipts.tests_passed(output)
        if "dynamic_lookup" in output or "warning:" in output:
            raise RuntimeError("strict positive contains global allowance or compiler/linker warning")
        self.inspect_executable(package, release, output)
        negatives = negative.qualify(self, Path(release["archives"][0]), package)
        return {"ir": census, "check": check, "release": release, "safe_tests": tests, "negatives": negatives}

    def inspect_executable(self, package, release, log):
        binaries = list((package / ".build").rglob("XbergPackageTests.xctest/Contents/MacOS/XbergPackageTests"))
        if len(binaries) != 1:
            raise RuntimeError("missing unique real XCTest executable")
        loads = self.command("actual-executable-loads", ["otool", "-L", binaries[0]], self.xberg, 0, 15)
        text = (loads / "stdout.log").read_text()
        if re_search_rust_dylib(text):
            raise RuntimeError("Rust dynamic library substituted for produced archive")
        linkmap = Path(self.env["SWIFT_GATE_LINK_MAP"])
        if not linkmap.is_file() or release["archives"][0] not in log or "libxberg_swift.a(" not in linkmap.read_text():
            raise RuntimeError("actual link map and verbose linker argv do not prove static archive")


def re_search_rust_dylib(text):
    return re.search(r"lib(?:xberg|alef)[^\s]*\.dylib", text) is not None


def harness_controls():
    root = Path(os.environ["GITHUB_WORKSPACE"])
    evidence = Path(os.environ.get("SWIFT_GATE_CONTROL_EVIDENCE", str(root / "consumer-controls-evidence")))
    suite = unittest.defaultTestLoader.discover(str(Path(__file__).parent), pattern="test_*.py")
    expected = 50
    actual = suite.countTestCases()
    if actual != expected:
        (evidence / "census-refusal.json").write_text(json.dumps({"expected": expected, "discovered": actual}))
        raise RuntimeError("hosted harness control census changed or examined zero cases")
    with (evidence / "unittest.stderr").open("x") as log:
        result = unittest.TextTestRunner(stream=log, verbosity=2).run(suite)
    sys.stderr.write((evidence / "unittest.stderr").read_text())
    census = {
        "expected": expected,
        "discovered": actual,
        "executed": result.testsRun,
        "failures": len(result.failures),
        "errors": len(result.errors),
        "ignored": len(result.skipped),
    }
    (evidence / "control-census.json").write_text(json.dumps(census, indent=2) + "\n")
    if result.testsRun != expected or result.skipped or not result.wasSuccessful():
        raise RuntimeError("required harness controls failed or were skipped")


def main():
    if sys.argv[1:] == ["--controls"]:
        return harness_controls()
    if sys.argv[1:]:
        raise ValueError("unknown hosted driver mode")
    gate = Gate()
    for name, repository, revision in [
        ("alef", "xberg-io/alef", os.environ["ALEF_REF"]),
        ("xberg", "xberg-io/xberg", os.environ["XBERG_REF"]),
    ]:
        gate.checkout(name, repository, revision)
    bootstrap.bootstrap(gate)
    gate.install_adapters()
    gate.configure_strict_flags()
    cli = gate.build_cli()
    result = gate.consumer(cli)
    (gate.evidence / "qualified-consumer.json").write_text(json.dumps(result, indent=2) + "\n")
    return None


if __name__ == "__main__":
    main()
