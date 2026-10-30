"""Bootstrap ordering and fail-closed toolchain policy at the owned Gate boundary."""

import tomllib
import unittest
from pathlib import Path

import bootstrap


class IdentityReachedError(RuntimeError):
    pass


class InstallFailureError(RuntimeError):
    pass


class RecordingGate:
    def __init__(self, fail_install=False):
        self.alef = Path("/owned/alef")
        self.env = {"RUSTUP_TOOLCHAIN": "1.99.1", "RUSTUP_AUTO_INSTALL": "1"}
        self.calls = []
        self.fail_install = fail_install

    def require_head(self, _root, _sha):
        pass

    def command(self, name, argv, cwd, budget, timeout):
        self.calls.append(
            {"name": name, "argv": argv, "cwd": cwd, "budget": budget, "timeout": timeout, "env": dict(self.env)}
        )
        if name == "rust-toolchain" and self.fail_install:
            raise InstallFailureError("explicit installer failed")
        if name == "rustup-identity":
            raise IdentityReachedError("stop before later bootstrap stages")


def through_identity(gate):
    try:
        bootstrap.bootstrap(gate)
    except IdentityReachedError:
        return
    raise AssertionError("bootstrap did not reach the identity boundary")


class BootstrapPolicy(unittest.TestCase):
    def test_pinned_install_precedes_identity_with_original_guarded_allowance(self):
        gate = RecordingGate()
        through_identity(gate)
        assert [row["name"] for row in gate.calls] == ["rust-toolchain", "rustup-identity"]
        install, identity = gate.calls
        declared = tomllib.loads((Path(bootstrap.__file__).resolve().parents[2] / "rust-toolchain.toml").read_text())[
            "toolchain"
        ]
        argv = install["argv"]
        assert argv[:3] == ["rustup", "toolchain", "install"]
        assert argv[3] == declared["channel"]
        assert argv[argv.index("--profile") + 1] == declared["profile"]
        components = [argv[index + 1] for index, flag in enumerate(argv) if flag == "--component"]
        targets = [argv[index + 1] for index, flag in enumerate(argv) if flag == "--target"]
        assert len(components) == len(set(components))
        assert set(components) == set(declared["components"])
        assert len(targets) == len(set(targets))
        assert set(targets) == set(declared["targets"])
        assert argv.count("--no-self-update") == 1
        assert (install["budget"], install["timeout"], install["cwd"]) == (2 * 1024**3, 600, gate.alef)
        assert (identity["argv"], identity["budget"], identity["timeout"]) == (["rustup", "--version"], 0, 10)

    def test_identity_and_explicit_install_disable_implicit_toolchain_install(self):
        gate = RecordingGate()
        through_identity(gate)
        assert gate.calls
        assert all(row["env"]["RUSTUP_AUTO_INSTALL"] == "0" for row in gate.calls)
        assert all(row["env"]["RUSTUP_TOOLCHAIN"] == "1.99.1" for row in gate.calls)
        assert gate.env["RUSTUP_AUTO_INSTALL"] == "0"

    def test_explicit_install_failure_stops_before_manager_identity(self):
        gate = RecordingGate(fail_install=True)
        caught = False
        try:
            bootstrap.bootstrap(gate)
        except InstallFailureError as error:
            caught = str(error) == "explicit installer failed"
        except IdentityReachedError:
            caught = False
        assert caught, "manager identity was reached before the failed explicit installer"
        assert [row["name"] for row in gate.calls] == ["rust-toolchain"]
