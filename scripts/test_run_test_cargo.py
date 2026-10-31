"""Exercise the canonical task against a recording Cargo boundary, without compiling."""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from run_test_cargo import isolated_command, main

REPO = Path(__file__).resolve().parents[1]


def recording_cargo(root):
    (root / "record_cargo.py").write_text(
        "import json, os, sys\n"
        "from pathlib import Path\n"
        "Path(os.environ['CARGO_TEST_RECEIPT']).write_text(json.dumps({\n"
        "    'argv': sys.argv[1:],\n"
        "    'platform': sys.platform,\n"
        "    'target': os.environ.get('CARGO_TARGET_DIR'),\n"
        "    'sentinel': os.environ.get('CARGO_TEST_SENTINEL'),\n"
        "}))\n"
        "sys.exit(int(os.environ['CARGO_TEST_EXIT']))\n"
    )


def prepare_fixture(root, wrapper_source, trace_target=False):
    config = tomllib.loads((REPO / "mise.toml").read_text())
    command = config["tasks"]["test:run"]["run"]
    assert command.count("cargo ") == 1, "expected one Cargo executable boundary"
    # Substitute only the executable; a Python recorder also works without Unix shebangs. ~keep
    command = command.replace("cargo ", "python record_cargo.py ", 1)
    (root / "mise.toml").write_text(
        "[settings]\ntask.run_auto_install=false\n[tools]\npython="
        + json.dumps(config["tools"]["python"])
        + '\n[tasks."test:run"]\nrun='
        + json.dumps(command)
        + "\n"
    )
    wrapper = REPO / "scripts/run_test_cargo.py"
    if wrapper.exists():
        (root / "scripts").mkdir()
        destination = root / "scripts/run_test_cargo.py"
        if trace_target:
            source = wrapper.read_text() if wrapper_source is None else wrapper_source
            anchor = "    arguments, environment = isolated_command(sys.argv[1:], os.environ)\n"
            assert source.count(anchor) == 1, "diagnostic must wrap exactly the real isolation boundary"
            diagnostic = (
                "    import json\n"
                "    from pathlib import Path\n"
                "    entry = {'platform': sys.platform, 'argv': sys.argv[1:], "
                "'target': os.environ.get('CARGO_TARGET_DIR')}\n"
                + anchor
                + "    diagnostic = {'entry': entry, 'forwarded': {'argv': arguments, "
                "'target': environment.get('CARGO_TARGET_DIR')}}\n"
                + f"    Path({str(root / 'wrapper-boundary.json')!r}).write_text(json.dumps(diagnostic))\n"
            )
            destination.write_text(source.replace(anchor, diagnostic, 1))
        elif wrapper_source is None:
            shutil.copyfile(wrapper, destination)
        else:
            destination.write_text(wrapper_source)


def invoke_task(arguments=(), target="outer target", exit_code=0, wrapper_source=None):
    parent_environment = dict(os.environ)
    with tempfile.TemporaryDirectory(prefix="alef-test-runner-") as directory:
        root = Path(directory).resolve()
        recording_cargo(root)
        receipt = root / "receipt.json"
        trace_target = target == "/absolute target/with spaces"
        prepare_fixture(root, wrapper_source, trace_target=trace_target)
        environment = {
            **parent_environment,
            "MISE_TRUSTED_CONFIG_PATHS": str(root),
            "MISE_CEILING_PATHS": str(root.parent),
            "MISE_GLOBAL_CONFIG_FILE": str(root / "no-global.toml"),
            "CARGO_TEST_RECEIPT": str(receipt),
            "CARGO_TEST_SENTINEL": "preserve unrelated environment",
            "CARGO_TEST_EXIT": str(exit_code),
        }
        environment.pop("MISE_ENV", None)
        environment.pop("CARGO_TARGET_DIR", None)
        if target is not None:
            environment["CARGO_TARGET_DIR"] = target
        result = subprocess.run(
            ["mise", "run", "test:run", "--", *arguments],
            cwd=root,
            env=environment,
            capture_output=True,
            text=True,
            timeout=15,
            check=False,
        )
        assert receipt.is_file(), f"recording Cargo was not invoked: {result.stderr}"
        captured = json.loads(receipt.read_text())
        if trace_target:
            trace = root / "wrapper-boundary.json"
            assert trace.is_file(), "diagnostic wrapper did not reach its forwarding boundary"
            captured["diagnostic"] = {
                "parent_platform": sys.platform,
                "parent_target": target,
                "mise_input_target": environment.get("CARGO_TARGET_DIR"),
                "wrapper": json.loads(trace.read_text()),
            }
    assert dict(os.environ) == parent_environment, "task invocation changed the parent environment"
    return (result, captured)


def canonical_arguments(*arguments):
    prefix = ["--config", "profile.test.package.alef.opt-level=1"] if sys.platform == "darwin" else []
    return [*prefix, "test", "--workspace", *arguments]


class CargoTestRunner(unittest.TestCase):
    def test_windows_waits_for_cargo_and_exits_with_its_failure_status(self):
        arguments = ["cargo", "test", "--workspace"]
        expected, environment = isolated_command(arguments, os.environ)
        windows = SimpleNamespace(argv=["run_test_cargo.py", *arguments], platform="win32")
        with (
            mock.patch("run_test_cargo.sys", windows),
            mock.patch("run_test_cargo.os.execvpe") as overlay,
            mock.patch("subprocess.run", return_value=subprocess.CompletedProcess(expected, 101)) as waited,
        ):
            try:
                main()
            except SystemExit as error:
                exit_status = error.code
            else:
                raise AssertionError("Windows must wait for Cargo and propagate its failure status")
            assert exit_status == 101
            overlay.assert_not_called()
            waited.assert_called_once_with(expected, env=environment, check=False)

    def test_outer_target_is_passed_as_cargo_argument_without_reaching_fixtures(self):
        result, captured = invoke_task()
        assert result.returncode == 0, result.stderr
        assert captured["target"] is None
        assert captured["argv"] == canonical_arguments("--target-dir", "outer target")
        assert captured["sentinel"] == "preserve unrelated environment"

    def test_absent_target_preserves_cargo_default_selection(self):
        result, captured = invoke_task(target=None)
        assert result.returncode == 0, result.stderr
        assert captured["target"] is None
        assert captured["argv"] == canonical_arguments()

    def test_target_values_are_forwarded_as_one_unmodified_argument(self):
        for target in ["relative/target", "/absolute target/with spaces", "unicode-λ", ""]:
            with self.subTest(target=target):
                result, captured = invoke_task(target=target)
                assert result.returncode == 0, result.stderr
                assert captured["target"] is None
                expected = canonical_arguments("--target-dir", target)
                assert captured["argv"] == expected, json.dumps(
                    {"expected_argv": expected, "received": captured}, ensure_ascii=True, sort_keys=True
                )

    def test_both_explicit_target_forms_override_the_outer_target(self):
        for arguments in [["--target-dir", "explicit target"], ["--target-dir=explicit target"]]:
            with self.subTest(arguments=arguments):
                result, captured = invoke_task(arguments)
                assert result.returncode == 0, result.stderr
                assert captured["target"] is None
                assert captured["argv"] == canonical_arguments(*arguments)

    def test_harness_target_option_does_not_override_the_cargo_target(self):
        arguments = ["--lib", "selected_test", "--", "--target-dir", "harness value", "--exact"]
        result, captured = invoke_task(arguments)
        assert result.returncode == 0, result.stderr
        expected = canonical_arguments(
            "--lib", "selected_test", "--target-dir", "outer target", "--", "--target-dir", "harness value", "--exact"
        )
        assert captured["argv"] == expected

    def test_every_requested_cargo_and_harness_argument_is_forwarded(self):
        arguments = ["--locked", "--no-fail-fast", "--test", "named target", "--", "--nocapture", "--exact"]
        result, captured = invoke_task(arguments, target=None)
        assert result.returncode == 0, result.stderr
        assert captured["argv"] == canonical_arguments(*arguments)

    def test_cargo_failure_exit_status_is_preserved(self):
        cargo_failure = 101
        result, captured = invoke_task(exit_code=cargo_failure)
        assert result.returncode == cargo_failure, result.stderr
        assert captured["argv"] == canonical_arguments("--target-dir", "outer target")

    def test_command_construction_does_not_mutate_its_inputs(self):
        arguments = ["cargo", "test", "--workspace", "--", "--exact"]
        environment = {"CARGO_TARGET_DIR": "outer target", "OTHER": "unchanged"}
        command, child = isolated_command(arguments, environment)
        assert arguments == ["cargo", "test", "--workspace", "--", "--exact"]
        assert environment == {"CARGO_TARGET_DIR": "outer target", "OTHER": "unchanged"}
        assert command == ["cargo", "test", "--workspace", "--target-dir", "outer target", "--", "--exact"]
        assert child == {"OTHER": "unchanged"}

    def test_recording_boundary_detects_a_hidden_workspace_narrowing_mutant(self):
        source = (REPO / "scripts/run_test_cargo.py").read_text()
        mutant = source.replace(
            "os.execvpe(arguments[0], arguments, environment)",
            'os.execvpe(arguments[0], [*arguments, "--lib"], environment)',
        ).replace(
            "subprocess.run(arguments, env=environment, check=False)",
            'subprocess.run([*arguments, "--lib"], env=environment, check=False)',
        )
        assert source != mutant
        result, captured = invoke_task(target=None, wrapper_source=mutant)
        assert result.returncode == 0, result.stderr
        assert captured["argv"] == canonical_arguments("--lib")
        try:
            assert captured["argv"] == canonical_arguments()
        except AssertionError:
            pass
        else:
            raise AssertionError("hidden --lib escaped the argument contract")


if __name__ == "__main__":
    unittest.main()
