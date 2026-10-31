"""Keep the outer Cargo target selection out of temporary artifact fixtures (#756)."""

import os
import subprocess
import sys


def isolated_command(arguments, environment):
    child_environment = dict(environment)
    target = child_environment.pop("CARGO_TARGET_DIR", None)
    separator = arguments.index("--") if "--" in arguments else len(arguments)
    explicit_target = any(
        argument == "--target-dir" or argument.startswith("--target-dir=") for argument in arguments[:separator]
    )
    if target is not None and not explicit_target:
        arguments = [*arguments[:separator], "--target-dir", target, *arguments[separator:]]
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
