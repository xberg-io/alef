"""Exercise the real version task with a recording Cargo process boundary."""

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
VERSION_FILES = (
    "Cargo.toml",
    "crates/alef-component-abi/Cargo.toml",
    "crates/alef-component-runtime/Cargo.toml",
    "alef.toml",
    "src/core/template_versions.rs",
)


class SetVersionWorkspaceTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="alef-version-owner-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for relative in VERSION_FILES:
            destination = self.root / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, destination)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        cargo = self.bin / "cargo"
        cargo.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, sys\n"
            "with open(os.environ['VERSION_TASK_LOG'], 'a') as stream:\n"
            "    stream.write(json.dumps({'args': sys.argv[1:], "
            "'target': os.environ.get('CARGO_TARGET_DIR')}) + '\\n')\n"
        )
        cargo.chmod(0o755)

    def run_task(self, target=None):
        log = self.root / "cargo.jsonl"
        environment = dict(os.environ, usage_version="0.115.16", VERSION_TASK_LOG=str(log))
        environment["PATH"] = str(self.bin) + os.pathsep + environment["PATH"]
        environment.pop("CARGO_TARGET_DIR", None)
        if target is not None:
            environment["CARGO_TARGET_DIR"] = target
        result = subprocess.run(
            ["bash", str(ROOT / "mise-tasks/set-version.sh")],
            cwd=self.root,
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )
        assert result.returncode == 0, result.stderr
        records = [json.loads(line) for line in log.read_text().splitlines()]
        assert len(records) == 2
        assert records[0]["args"] == [
            "run",
            "--quiet",
            "--",
            "schema",
            "--schema-version",
            "0.115.16",
            "--output",
            "schemas/alef.schema.json",
        ]
        return records

    def test_explicit_schema_target_is_preserved(self):
        target = str(self.root / "owned target")
        records = self.run_task(target)
        assert records[0]["target"] == target
        assert records[1]["target"] == target

    def test_lock_update_retains_registry_dependency_selections(self):
        records = self.run_task()
        assert records[1]["args"] == ["update", "--workspace"]

    def test_default_schema_target_and_all_source_versions_remain_supported(self):
        records = self.run_task()
        assert records[0]["target"] == "target/schema-task"
        assert records[1]["target"] is None
        for relative in VERSION_FILES:
            contents = (self.root / relative).read_text()
            assert ('"0.115.16"' if relative.endswith(".toml") else '"v0.115.16"') in contents


if __name__ == "__main__":
    unittest.main()
