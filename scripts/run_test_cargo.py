"""Keep the outer Cargo target selection out of temporary artifact fixtures (#756)."""

import os
import subprocess
import sys
from pathlib import Path


def isolated_command(arguments, environment):
    child_environment = dict(environment)
    target = child_environment.pop("CARGO_TARGET_DIR", None)
    separator = arguments.index("--") if "--" in arguments else len(arguments)
    explicit_target = any(
        argument == "--target-dir" or argument.startswith("--target-dir=") for argument in arguments[:separator]
    )
    if target is not None and not explicit_target:
        arguments = [*arguments[:separator], "--target-dir", target, *arguments[separator:]]
        separator += 2
    for index, argument in enumerate(arguments[:separator]):
        if argument == "--target-dir" and index + 1 < len(arguments):
            child_environment["ALEF_TEST_TARGET_DIR"] = str(Path(arguments[index + 1]).resolve())
            break
        if argument.startswith("--target-dir="):
            child_environment["ALEF_TEST_TARGET_DIR"] = str(Path(argument.partition("=")[2]).resolve())
            break
    return arguments, child_environment


def main():
    arguments, environment = isolated_command(sys.argv[1:], os.environ)
    if not arguments:
        raise SystemExit("expected the Cargo test command")
    if sys.platform == "win32":
        raise SystemExit(subprocess.run(arguments, env=environment, check=False).returncode)
    os.execvpe(arguments[0], arguments, environment)


if __name__ == "__main__":
    main()
