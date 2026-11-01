"""Bind extracted Bash workflow controls to their already-selected Python interpreter."""

import json
import os
import re
import shlex
import sys
from pathlib import Path


def workflow_environment(cwd, poisoned_python):
    environment = os.environ.copy()
    if poisoned_python:
        poisoned = cwd / "poisoned tools"
        poisoned.mkdir()
        shim = poisoned / "python3"
        shim.write_text("#!/bin/sh\necho POISONED_PYTHON_SHIM >&2\nexit 97\n", newline="\n")
        shim.chmod(0o755)
        environment["PATH"] = str(poisoned) + os.pathsep + environment["PATH"]
    tools = cwd / "managed interpreter"
    tools.mkdir()
    calls = cwd / "python invocations"
    interpreter = tools / "python3"
    interpreter.write_text(
        "#!/bin/sh\n"
        f"printf 'native-python\\n' >> {shlex.quote(calls.as_posix())}\n"
        f'exec {shlex.quote(Path(sys.executable).as_posix())} "$@"\n',
        newline="\n",
    )
    interpreter.chmod(0o755)
    environment["PATH"] = str(tools) + os.pathsep + environment["PATH"]
    return environment, calls


def assert_archive_digests(output, expected):
    digests = json.loads(output)
    assert set(digests) == set(expected)
    assert all(re.fullmatch("[0-9a-f]{64}", value) for value in digests.values())
