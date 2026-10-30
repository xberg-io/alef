"""Pinned formatter setup through actual bootstrap at its owned command boundary."""

import json
import os
import shutil
import tempfile
import unittest
from pathlib import Path

import bootstrap

DECLARED = {"github:Goldziher/poly": "0.31.2", "github:DevinR528/cargo-sort": "2.1.4"}


class FormatterGate:
    def __init__(self, root, wrong_version=False, wrong_path=False, invalid_report=None):
        root = root.resolve(strict=True)
        self.alef = root / "alef"
        self.xberg = root / "xberg"
        self.evidence = root / "evidence"
        self.tools = self.evidence / "tools"
        self.bin = self.tools / "bin"
        self.runner_temp = root / "runner-temp"
        self.ambient = root / "ambient"
        self.env = {
            "PATH": str(self.ambient),
            "CARGO_HOME": str(root / "cargo-home"),
            "RUNNER_TEMP": str(self.runner_temp),
        }
        self.real_cargo = "/owned/cargo"
        self.python = "/owned/python"
        self.paths = []
        self.calls = []
        self.wrong_version = wrong_version
        self.wrong_path = wrong_path
        self.invalid_report = invalid_report
        for path in [self.alef, self.xberg, self.evidence, self.bin, self.runner_temp, self.ambient]:
            path.mkdir(parents=True, exist_ok=True)
        for path in [root / "sysroot/lib", root / "brew-cache"]:
            path.mkdir(parents=True, exist_ok=True)
        (self.xberg / "mise.toml").write_text('[tools]\n"github:Goldziher/poly" = "0.31.2"\n')
        (self.alef / "mise.toml").write_text('[tools]\n"github:DevinR528/cargo-sort" = "2.1.4"\n')
        for checkout in [self.alef, self.xberg]:
            (checkout / "mise.lock").write_text("# owned boundary fixture\n")
        executable(self.ambient / "mise")
        self.selected = {}

    def require_head(self, _root, _sha):
        pass

    def publish_paths(self):
        pass

    def which(self, tool):
        if tool != "clang-format":
            return shutil.which(tool, path=self.env["PATH"]) or "/owned/" + tool
        binary = self.tools / "clang-format/bin/clang-format"
        if self.wrong_path:
            return "/foreign/clang-format"
        if binary.is_file() and self.env["PATH"].split(os.pathsep)[0] == str(binary.parent):
            return str(binary)
        return None

    def command(self, name, argv, cwd, budget, timeout):
        self.calls.append({"name": name, "argv": argv, "cwd": cwd, "budget": budget, "timeout": timeout})
        result = self.evidence / name
        result.mkdir()
        output = ""
        if name == "qualified-macos-deployment-target":
            output = "MACOSX_DEPLOYMENT_TARGET=11.0"
        elif name == "qualified-rust-sysroot":
            output = str(self.alef.parent / "sysroot")
        elif name == "actual-homebrew-cache":
            output = str(self.alef.parent / "brew-cache")
        elif name == "mise-identity":
            output = "2026.10.24 macos-arm64 (2026-10-29)"
        elif name.startswith("install-") and argv[:2] == ["mise", "install"]:
            tool, version = argv[-1].rsplit("@", 1)
            program = name.removeprefix("install-")
            installs = Path(self.env["MISE_DATA_DIR"]) / "installs"
            self.selected[program] = executable(installs / tool.replace(":", "-").replace("/", "-") / version / program)
        elif name.startswith("mise-which-"):
            output = str(self.selected[name.removeprefix("mise-which-")])
        elif name.startswith("version-"):
            output = {
                "poly": "poly 0.31.2 (release build v0.31.2, release)",
                "cargo-sort": "cargo-sort 2.1.4",
                "rustc": "rustc 1.99.1",
                "cargo": "cargo 1.99.1",
                "clang-format": "clang-format version " + ("18.1.8" if self.wrong_version else "22.1.8"),
            }[name.removeprefix("version-")]
        elif name == "install-clang-format":
            binary = self.tools / "clang-format/bin/clang-format"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"owned boundary fixture, not a native executable")
            report = self.evidence / "clang-format-install.json"
            download = {
                "url": "https://files.pythonhosted.org/fixture.whl",
                "archive_info": {"hashes": {"sha256": "a" * 64}},
            }
            if self.invalid_report == "digest":
                download["archive_info"]["hashes"]["sha256"] = "not-a-sha256"
            elif self.invalid_report == "host":
                download["url"] = "https://files.pythonhosted.org.attacker.invalid/fixture.whl"
            elif self.invalid_report == "suffix":
                download["url"] = "https://files.pythonhosted.org/fixture.tar.gz"
            report.write_text(
                json.dumps(
                    {
                        "install": [
                            {"metadata": {"name": "clang-format", "version": "22.1.8"}, "download_info": download}
                        ]
                    }
                )
            )
        (result / "stdout.log").write_text(output + "\n")
        (result / "stderr.log").write_text("")
        return result


def executable(path):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"owned boundary fixture, not a native executable")
    path.chmod(0o755)
    return path


class FormatterBootstrap(unittest.TestCase):
    def test_missing_ambient_formatter_is_installed_pinned_and_selected_before_strict_check(self):
        with tempfile.TemporaryDirectory() as directory:
            gate = FormatterGate(Path(directory))
            try:
                bootstrap.bootstrap(gate)
            except RuntimeError as error:
                self.fail("pinned formatter setup did not complete: " + str(error))
            calls = {row["name"]: row for row in gate.calls}
            install = calls["install-clang-format"]
            assert "clang-format==22.1.8" in install["argv"]
            assert "--only-binary=:all:" in install["argv"]
            assert install["argv"][install["argv"].index("--index-url") + 1] == "https://pypi.org/simple"
            assert (install["budget"], install["timeout"]) == (512 * 1024**2, 600)
            tool = json.loads((gate.evidence / "clang-format-tool.json").read_text())
            assert tool["selected"] == gate.which("clang-format")
            assert tool["version"] == "22.1.8"

    def test_wrong_formatter_version_is_rejected_after_actual_version_command(self):
        with tempfile.TemporaryDirectory() as directory:
            gate = FormatterGate(Path(directory), wrong_version=True)
            try:
                bootstrap.bootstrap(gate)
            except RuntimeError as error:
                message = str(error)
            else:
                raise AssertionError("wrong formatter version was accepted")
            assert message == "installed clang-format version differs", message
            assert gate.calls[-1]["name"] == "version-clang-format"

    def test_foreign_path_cannot_satisfy_the_pinned_formatter_setup(self):
        for scenario in ["path", "digest", "host", "suffix"]:
            with self.subTest(scenario=scenario), tempfile.TemporaryDirectory() as directory:
                gate = FormatterGate(Path(directory), wrong_path=scenario == "path", invalid_report=scenario)
                try:
                    bootstrap.bootstrap(gate)
                except RuntimeError as error:
                    message = str(error)
                else:
                    raise AssertionError("foreign formatter executable was accepted")
                expected = (
                    "pinned clang-format executable is not selected on the generation PATH"
                    if scenario == "path"
                    else "official clang-format wheel provenance missing"
                )
                assert message == expected, message
                expected_stage = "install-clang-format" if scenario == "path" else "version-clang-format"
                assert gate.calls[-1]["name"] == expected_stage
