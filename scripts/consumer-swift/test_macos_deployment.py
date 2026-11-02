"""Bootstrap targets the shipped Swift package floor for Rust and native code alike."""

import json
import tempfile
import unittest
from pathlib import Path

import bootstrap
from assertions import raises
from test_formatter_bootstrap import FormatterGate


class DeploymentGate(FormatterGate):
    def __init__(self, root, report=None, diagnostics=""):
        super().__init__(root)
        self.report = report
        self.diagnostics = diagnostics
        self.observed = []

    def command(self, name, argv, cwd, budget, timeout):
        self.observed.append({"name": name, "env": dict(self.env)})
        result = super().command(name, argv, cwd, budget, timeout)
        if name == "qualified-macos-deployment-target" and self.report is not None:
            (result / "stdout.log").write_text(self.report)
        if name == "qualified-macos-deployment-target":
            (result / "stderr.log").write_text(self.diagnostics)
        return result


class MacosDeployment(unittest.TestCase):
    def test_missing_native_minimum_is_the_swift_package_floor_and_rust_selects_it(self):
        with tempfile.TemporaryDirectory() as directory:
            gate = DeploymentGate(Path(directory))
            gate.env["CFLAGS"] = "-O2"
            config = Path(gate.env["CARGO_HOME"]) / "config.toml"
            config.parent.mkdir()
            original = b"[build]\njobs = 1\n[env]\nCFLAGS_x86_64_pc_windows_gnu = '-Wa,-mbig-obj'\n"
            config.write_bytes(original)
            bootstrap.bootstrap(gate)
            assert gate.env.get("MACOSX_DEPLOYMENT_TARGET") == "13.0"
            assert gate.env["CFLAGS"] == "-O2"
            assert config.read_bytes() == original
            query = [row for row in gate.calls if row["name"] == "qualified-macos-deployment-target"]
            assert len(query) == 1
            assert query[0]["argv"] == ["rustup", "run", "1.99.1", "rustc", "--print=deployment-target"]
            assert (query[0]["cwd"], query[0]["budget"], query[0]["timeout"]) == (gate.alef, 0, 15)
            names = [row["name"] for row in gate.calls]
            assert names.index("qualified-macos-deployment-target") < names.index("install-cargo-sort")
            for row in gate.observed[names.index("qualified-macos-deployment-target") + 1 :]:
                assert row["env"]["MACOSX_DEPLOYMENT_TARGET"] == "13.0"
            assert json.loads((gate.evidence / "macos-deployment-target.json").read_text()) == {
                "requested": None,
                "swift_package_floor": "13.0",
                "rust_selected": "13.0",
                "effective": "13.0",
            }

    def test_explicit_ambient_target_is_preserved_and_queried_without_replacement(self):
        with tempfile.TemporaryDirectory() as directory:
            gate = DeploymentGate(Path(directory), report="MACOSX_DEPLOYMENT_TARGET=13.0.0\n")
            gate.env["MACOSX_DEPLOYMENT_TARGET"] = "13"
            bootstrap.bootstrap(gate)
            assert gate.env["MACOSX_DEPLOYMENT_TARGET"] == "13"
            rows = [row for row in gate.observed if row["name"] == "qualified-macos-deployment-target"]
            assert len(rows) == 1
            assert rows[0]["env"]["MACOSX_DEPLOYMENT_TARGET"] == "13"

    def test_invalid_or_mismatched_rust_selection_stops_before_native_cargo_tools(self):
        scenarios = [
            {
                "requested": "11.0",
                "message": "explicit macOS deployment target differs from the Swift package floor",
                "no_query": True,
            },
            {
                "report": "MACOSX_DEPLOYMENT_TARGET=11.0\n",
                "message": "qualified Rust macOS deployment target differs from the Swift package floor",
            },
            {
                "root_manifest": "platforms: [.macOS(.v14)]\n",
                "message": "shipped Swift manifests must declare one shared macOS platform floor",
                "no_query": True,
            },
            {
                "root_manifest": "platforms: [.iOS(.v16)]\n",
                "message": "Swift manifest must declare exactly one macOS platform floor: Package.swift",
                "no_query": True,
            },
            {
                "report": "IPHONEOS_DEPLOYMENT_TARGET=11.0\n",
                "message": "qualified Rust macOS deployment target is invalid or has diagnostics",
            },
            {
                "diagnostics": "warning: invalid target",
                "message": "qualified Rust macOS deployment target is invalid or has diagnostics",
            },
            {
                "cargo_target": "x86_64-apple-darwin",
                "message": "explicit Cargo build target requires deployment-target review",
                "no_query": True,
            },
            {
                "config": "[build]\ntarget = 'x86_64-apple-darwin'\n",
                "message": "Cargo configuration build target requires deployment-target review",
                "no_query": True,
            },
            {
                "config": "[build]\ntarget = [\n",
                "message": "unreadable or malformed Cargo configuration requires deployment-target review",
                "no_query": True,
            },
            {
                "config": "[env]\nMACOSX_DEPLOYMENT_TARGET = { value = '26.5', force = true }\n",
                "message": "Cargo configuration deployment environment requires review",
                "no_query": True,
            },
            {
                "config": "[env]\nCARGO_BUILD_TARGET = 'x86_64-apple-darwin'\n",
                "message": "Cargo configuration deployment environment requires review",
                "no_query": True,
            },
            {
                "relative_home": True,
                "message": "relative or empty Cargo home requires deployment-target review",
                "no_query": True,
            },
        ]
        for scenario in scenarios:
            with tempfile.TemporaryDirectory() as directory:
                gate = DeploymentGate(
                    Path(directory),
                    report=scenario.get("report"),
                    diagnostics=scenario.get("diagnostics", ""),
                )
                if "root_manifest" in scenario:
                    (gate.xberg / "Package.swift").write_text(scenario["root_manifest"])
                if "requested" in scenario:
                    gate.env["MACOSX_DEPLOYMENT_TARGET"] = scenario["requested"]
                if "cargo_target" in scenario:
                    gate.env["CARGO_BUILD_TARGET"] = scenario["cargo_target"]
                if "config" in scenario:
                    config = Path(gate.env["CARGO_HOME"]) / "config.toml"
                    config.parent.mkdir()
                    config.write_text(scenario["config"])
                if scenario.get("relative_home"):
                    gate.env["CARGO_HOME"] = "relative-cargo-home"
                try:
                    bootstrap.bootstrap(gate)
                except RuntimeError as error:
                    message = str(error)
                else:
                    raise AssertionError("invalid deployment selection was accepted")
                assert message == scenario["message"], message
                names = [row["name"] for row in gate.calls]
                assert "install-cargo-sort" not in names
                if scenario.get("no_query"):
                    assert "qualified-macos-deployment-target" not in names

    def test_swift_package_floor_reads_each_platform_spelling(self):
        with tempfile.TemporaryDirectory() as directory:
            xberg = Path(directory)
            (xberg / "packages/swift").mkdir(parents=True)
            for declared, floor in [(".v14", "14.0"), (".v10_15", "10.15"), ('"13.4"', "13.4")]:
                (xberg / "packages/swift/Package.swift").write_text(f"platforms: [.macOS({declared}), .iOS(.v16)]")
                assert bootstrap.swift_package_macos_floor(xberg) == floor
            for declared, mirrored, floor in [
                (".v14", '"14.0"', "14.0"),
                (".v14", '"14.0.0"', "14.0"),
                ('"13.4"', '"13.4.0"', "13.4"),
                ('"13.4.1"', '"13.4.1"', "13.4.1"),
            ]:
                with self.subTest(declared=declared, mirrored=mirrored):
                    (xberg / "packages/swift/Package.swift").write_text(f"platforms: [.macOS({declared})]")
                    (xberg / "Package.swift").write_text(f"platforms: [.macOS({mirrored})]")
                    assert bootstrap.swift_package_macos_floor(xberg) == floor
            (xberg / "Package.swift").write_text('platforms: [.macOS("13.4.2")]')
            with raises(RuntimeError, "must declare one shared macOS platform floor"):
                bootstrap.swift_package_macos_floor(xberg)
            for manifest in [xberg / "packages/swift/Package.swift", xberg / "Package.swift"]:
                manifest.unlink()
            with raises(RuntimeError, "must declare one shared macOS platform floor"):
                bootstrap.swift_package_macos_floor(xberg)
