"""Reject zero-work and incomplete Swift ownership qualification logs."""

import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/ci/run-swift-owned-vector-inputs.sh"
CASES = ("unique", "moving-unique", "clone-reuse", "clone-async-reuse", "moving-state")
SUMMARY = "test result: ok. 1 passed; 0 failed; 0 ignored; 42 filtered out; finished in 1s"


def healthy_log():
    return "\n".join([*(f"owned_vector_verified={case}" for case in CASES), SUMMARY, ""])


class SwiftOwnedVectorGate(unittest.TestCase):
    def check_log(self, log):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "runtime.log"
            path.write_text(log)
            return subprocess.run(
                ["bash", str(SCRIPT), "--check-log", str(path)], capture_output=True, text=True, check=False
            )

    def test_accepts_one_executed_test_and_all_five_runtime_cases(self):
        assert self.check_log(healthy_log()).returncode == 0

    def test_rejects_missing_or_ignored_executed_test(self):
        for summary in [
            "",
            "test result: ok. 0 passed; 0 failed; 0 ignored;",
            "test result: ok. 0 passed; 0 failed; 1 ignored;",
        ]:
            with self.subTest(summary=summary):
                assert self.check_log(healthy_log().replace(SUMMARY, summary)).returncode != 0

    def test_rejects_every_missing_runtime_case(self):
        for case in CASES:
            with self.subTest(case=case):
                assert self.check_log(healthy_log().replace(f"owned_vector_verified={case}\n", "")).returncode != 0

    def test_rejects_a_duplicate_runtime_case(self):
        assert self.check_log(healthy_log() + "owned_vector_verified=unique\n").returncode != 0

    def test_macos_workflow_calls_the_counted_runtime_gate(self):
        config = tomllib.loads((ROOT / "mise.toml").read_text())
        assert (
            config["tasks"]["gate:swift-owned-vector-inputs"]["run"]
            == "bash scripts/ci/run-swift-owned-vector-inputs.sh"
        )
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        name = "- name: Verify Swift opaque vector input ownership"
        assert workflow.count(name) == 1
        block = workflow.split(name, 1)[1].split("- name:", 1)[0]
        assert "if: runner.os == 'macOS'" in block
        assert "mise exec -- python scripts/ci/test_swift_owned_vector_gate.py" in block
        assert "SWIFT_EXEC" not in block
        assert "          mise run gate:swift-owned-vector-inputs\n" in block
        script = SCRIPT.read_text()
        assert "--ignored --exact --nocapture --test-threads=1" in script
        assert 'check_log "$log_file"' in script


if __name__ == "__main__":
    unittest.main()
