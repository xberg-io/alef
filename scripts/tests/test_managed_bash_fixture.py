"""Hermetic controls for native Git Bash selection in extracted workflow probes."""

import os
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

if __package__:
    from . import managed_python_fixture as fixture
    from . import test_github_release_state as controls
else:
    import managed_python_fixture as fixture
    import test_github_release_state as controls


def native_and_poisoned_bash(root):
    poisoned = root / "System32"
    poisoned.mkdir()
    if sys.platform == "win32":
        native = Path(os.environ["ALEF_TEST_BASH_EXECUTABLE"])
        shutil.copyfile(sys.executable, poisoned / "bash.exe")
    else:
        real_bash = shutil.which("bash")
        assert real_bash is not None
        native = root / "Git/usr/bin/bash.exe"
        native.parent.mkdir(parents=True)
        native.write_text(f'#!/bin/sh\nexec {shlex.quote(real_bash)} "$@"\n', newline="\n")
        native.chmod(0o755)
        (native.parent / "msys-2.0.dll").touch()
        poison = poisoned / "bash"
        poison.write_text("#!/bin/sh\necho POISONED_WSL_BASH >&2\nexit 96\n", newline="\n")
        poison.chmod(0o755)
    return native, poisoned


class NativeGitBashControls(unittest.TestCase):
    def test_explicit_native_shell_ignores_poisoned_wsl_path(self):
        with tempfile.TemporaryDirectory(prefix="selected native Bash ") as temporary:
            root = Path(temporary)
            native, poisoned = native_and_poisoned_bash(root)
            mise_selection = f'"{native}" -o errexit -c'
            for explicit in [True, False]:
                environment = {
                    "MISE_WINDOWS_DEFAULT_INLINE_SHELL_ARGS": mise_selection,
                    "PATH": str(poisoned) + os.pathsep + os.environ["PATH"],
                    "BASH": str(root / "Ruby/msys64/usr/bin/bash.exe"),
                }
                if explicit:
                    environment["ALEF_TEST_BASH_EXECUTABLE"] = str(native)
                with self.subTest(explicit=explicit), patch.dict(os.environ, environment):
                    if not explicit:
                        os.environ.pop("ALEF_TEST_BASH_EXECUTABLE", None)
                    with patch.object(fixture.sys, "platform", "win32"):
                        assert fixture.workflow_bash(os.environ) == str(native)
                        with patch.object(controls.subprocess, "run", wraps=subprocess.run) as launched:
                            result = controls.WorkflowBoundaryTest().guard()
                    assert result.returncode == 0, result.stdout + result.stderr
                    assert launched.call_count == 1
                    assert launched.call_args.args[0][0] == str(native)
                    controls.assert_archive_digests(result.stdout, controls.SPEC["assets"])

    def test_invalid_windows_shell_binding_refuses_before_any_spawn(self):
        with tempfile.TemporaryDirectory(prefix="invalid native Bash ") as temporary:
            root = Path(temporary)
            system32 = root / "System32/bash.exe"
            system32.parent.mkdir()
            system32.touch()
            system32.chmod(0o755)
            wrong_name = root / "Git/usr/bin/python.exe"
            wrong_name.parent.mkdir(parents=True)
            wrong_name.touch()
            wrong_name.chmod(0o755)
            (wrong_name.parent / "msys-2.0.dll").touch()
            native = wrong_name.with_name("bash.exe")
            native.touch()
            native.chmod(0o755)
            newline_native = str(root / "bad") + "\nname/bash.exe"
            for environment in invalid_bindings(root, native, system32, wrong_name, newline_native):
                with (
                    self.subTest(binding=environment),
                    patch.object(fixture.sys, "platform", "win32"),
                    patch.object(subprocess, "run", side_effect=AssertionError("invalid shell was spawned")) as launch,
                ):
                    expected = (
                        "single-line"
                        if environment.get("ALEF_TEST_BASH_EXECUTABLE") == newline_native
                        else "native Git Bash"
                    )
                    with controls.rejected(ValueError, expected):
                        fixture.workflow_bash(environment)
                    launch.assert_not_called()


def invalid_bindings(root, native, system32, wrong_name, newline_native):
    key = "MISE_WINDOWS_DEFAULT_INLINE_SHELL_ARGS"
    valid = f'"{native}" -o errexit -c'
    explicit = [
        "",
        "relative/bash.exe",
        str(root / "missing/bash.exe"),
        str(system32),
        str(wrong_name),
        str(newline_native),
    ]
    malformed = [
        "",
        '"unterminated',
        "relative/bash.exe -o errexit -c",
        f'"{root / "missing/bash.exe"}" -o errexit -c',
        f'"{system32}" -o errexit -c',
        f'"{wrong_name}" -o errexit -c',
        f'"{native}" -c',
        valid + " injected",
        valid + "; echo injected",
    ]
    return (
        [{}]
        + [{"ALEF_TEST_BASH_EXECUTABLE": value, key: valid} for value in explicit]
        + [{key: value} for value in malformed]
    )


if __name__ == "__main__":
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(NativeGitBashControls)
    assert suite.countTestCases() == 2
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    assert result.testsRun == 2
    assert not result.skipped
    raise SystemExit(0 if result.wasSuccessful() else 1)
