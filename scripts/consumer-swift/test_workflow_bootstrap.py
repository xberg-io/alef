"""Proposed source-contract controls; no bootstrap, network or hosted build runs here."""

import re
import unittest
from pathlib import Path

from assertions import raises

VERIFY = (
    "run: python3 gate/scripts/consumer-swift/entry.py --controls && python3 gate/scripts/consumer-swift/entry.py\n"
)


def require_mise_before_qualification(workflow):
    steps = re.split(r"(?m)^      - ", workflow)[1:]
    mise = [
        i
        for i, step in enumerate(steps)
        if "uses: xberg-io/actions/setup-mise@v1\n" in step and 'install: "false"\n' in step
    ]
    verify = [i for i, step in enumerate(steps) if VERIFY in step]
    if len(mise) != 1 or len(verify) != 1 or mise[0] >= verify[0]:
        raise ValueError("tool-free mise setup must precede the guarded qualification")
    if "ref: ${{ github.sha }}\n" not in steps[0] or "path: gate\n" not in steps[0]:
        raise ValueError("workflow harness checkout must use its immutable invocation commit")


def task_runner_references(text):
    return re.findall(r"go-task|--bootstrap-task|\"task\",|SWIFT_GATE_BOOTSTRAP|consumer-task", text)


class WorkflowBootstrap(unittest.TestCase):
    def workflow(self):
        return (Path(__file__).resolve().parents[2] / ".github/workflows/verify-xberg-swift.yaml").read_text()

    def test_actual_workflow_sets_up_mise_before_qualification(self):
        require_mise_before_qualification(self.workflow())

    def test_missing_late_and_tool_installing_mise_setup_are_rejected(self):
        source = self.workflow()
        prefix, *steps = re.split(r"(?m)^      - ", source)
        setup = next(step for step in steps if "setup-mise@v1" in step)
        without = [step for step in steps if step != setup]
        for mutated in [
            prefix + "      - ".join(["", *without]),
            prefix + "      - ".join(["", *without, setup]),
            source.replace('install: "false"\n', 'install: "true"\n'),
        ]:
            with self.subTest(mutated=mutated), raises(ValueError, "must precede the guarded qualification"):
                require_mise_before_qualification(mutated)

    def test_gate_sources_carry_no_go_task_bootstrap(self):
        sources = [Path(__file__).resolve().parents[2] / ".github/workflows/verify-xberg-swift.yaml"]
        sources += [path for path in Path(__file__).resolve().parent.glob("*.py") if path.name != Path(__file__).name]
        assert len(sources) > 10, sources
        found = {str(path): task_runner_references(path.read_text()) for path in sources}
        assert not any(found.values()), found
        assert task_runner_references('["task", "swift:test"]') == ['"task",']
