"""Locked mise tool installation at the owned Gate command boundary."""

import json
import os
import tempfile
import unittest
from pathlib import Path

import bootstrap
from test_formatter_bootstrap import FormatterGate, executable


class MiseGate(FormatterGate):
    def __init__(self, root, foreign=None, version_output=None):
        super().__init__(root)
        self.foreign = foreign
        self.version_output = version_output

    def command(self, name, argv, cwd, budget, timeout):
        result = super().command(name, argv, cwd, budget, timeout)
        program = name.removeprefix("mise-which-")
        if name.startswith("mise-which-") and program == self.foreign:
            stray = executable(self.ambient / "stray" / program)
            (result / "stdout.log").write_text(str(stray) + "\n")
        if self.version_output and name == "version-" + self.version_output[0]:
            (result / "stdout.log").write_text(self.version_output[1] + "\n")
        return result


def failure(gate):
    try:
        bootstrap.bootstrap(gate)
    except RuntimeError as error:
        return str(error)
    raise AssertionError("bootstrap accepted an unqualified mise tool")


class MiseBootstrap(unittest.TestCase):
    def test_each_tool_installs_locked_from_its_declaring_checkout_into_a_private_data_dir(self):
        with tempfile.TemporaryDirectory() as directory:
            gate = MiseGate(Path(directory))
            bootstrap.bootstrap(gate)
            calls = {row["name"]: row for row in gate.calls}
            assert calls["install-poly"]["argv"] == ["mise", "install", "--locked", "github:Goldziher/poly@0.31.2"]
            assert calls["install-poly"]["cwd"] == gate.xberg
            assert calls["install-cargo-sort"]["argv"] == [
                "mise",
                "install",
                "--locked",
                "github:DevinR528/cargo-sort@2.1.4",
            ]
            assert calls["install-cargo-sort"]["cwd"] == gate.alef
            data = gate.runner_temp / "consumer-mise/data"
            assert gate.env["MISE_DATA_DIR"] == str(data)
            assert gate.env["MISE_LOCKED"] == "1"
            assert gate.env["MISE_TRUSTED_CONFIG_PATHS"].split(os.pathsep) == [str(gate.alef), str(gate.xberg)]
            receipt = json.loads((gate.evidence / "mise-tools.json").read_text())
            assert receipt["mise"] == str((gate.ambient / "mise").resolve())
            assert [row["tool"] for row in receipt["tools"]] == ["github:Goldziher/poly", "github:DevinR528/cargo-sort"]
            for row, program in zip(receipt["tools"], ["poly", "cargo-sort"], strict=True):
                assert row["selected"] == gate.which(program)
                assert Path(row["selected"]).is_relative_to(data)
                assert len(row["executable_sha256"]) == 64
            names = [row["name"] for row in gate.calls]
            assert names.index("install-cargo-sort") < names.index("native-dependencies")

    def test_tool_not_pinned_by_its_checkout_is_refused_before_any_install(self):
        for scenario in ["floating", "absent", "unlocked"]:
            with self.subTest(scenario=scenario), tempfile.TemporaryDirectory() as directory:
                gate = MiseGate(Path(directory))
                if scenario == "floating":
                    (gate.xberg / "mise.toml").write_text('[tools]\n"github:Goldziher/poly" = "latest"\n')
                elif scenario == "absent":
                    (gate.xberg / "mise.toml").write_text("[tools]\n")
                else:
                    (gate.xberg / "mise.lock").unlink()
                assert failure(gate) == "mise config must pin and lock generation tool: github:Goldziher/poly"
                assert not [row for row in gate.calls if row["name"].startswith("install-")]

    def test_selection_outside_the_private_data_dir_is_refused_before_path_change(self):
        with tempfile.TemporaryDirectory() as directory:
            gate = MiseGate(Path(directory), foreign="cargo-sort")
            message = failure(gate)
            assert message == "mise selected an executable outside its private data directory: cargo-sort", message
            assert "stray" not in gate.env["PATH"]
            assert gate.calls[-1]["name"] == "mise-which-cargo-sort"

    def test_installed_version_must_equal_the_declared_pin(self):
        with tempfile.TemporaryDirectory() as directory:
            gate = MiseGate(Path(directory), version_output=("poly", "poly 0.24.0"))
            assert failure(gate) == "installed tool version differs: poly"
            assert gate.calls[-1]["name"] == "version-poly"
            assert not (gate.evidence / "mise-tools.json").exists()
