"""Proposed source-contract controls; no bootstrap, network or hosted build runs here."""

import re
import unittest
from pathlib import Path

import driver
import entry
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


# Run 41187024458: every stage but the release build took 2060 s; the stages after it have 2925 s of
# timeouts. GitHub-hosted jobs stop at 6 h; setup, controls and upload need about 15 minutes.
OTHER_STAGES, LATER_TIMEOUTS, HOSTED_JOB_LIMIT, JOB_OVERHEAD = 2060, 2925, 360 * 60, 15 * 60


def require_time_budget(workflow, wall, release):
    minutes = re.findall(r"(?m)^    timeout-minutes: (\d+)$", workflow)
    if len(minutes) != 1:
        raise ValueError("expected one consumer job timeout")
    job = int(minutes[0]) * 60
    if not OTHER_STAGES + release + LATER_TIMEOUTS <= wall <= job - JOB_OVERHEAD <= HOSTED_JOB_LIMIT - JOB_OVERHEAD:
        raise ValueError("release timeout, session wall and job timeout do not nest")


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

    def test_release_timeout_fits_the_session_wall_and_the_hosted_job_limit(self):
        source = self.workflow()
        require_time_budget(source, entry.WALL["verify"], driver.RELEASE_BUILD_TIMEOUT)
        for mutated, wall, release in [
            (source, entry.WALL["verify"], 3600 + entry.WALL["verify"]),
            (source.replace("timeout-minutes: 275", "timeout-minutes: 180"), entry.WALL["verify"], 3600),
            (source.replace("timeout-minutes: 275", "timeout-minutes: 400"), 23000, 3600),
            (source.replace("    timeout-minutes: 275\n", ""), entry.WALL["verify"], 3600),
        ]:
            with self.subTest(wall=wall, release=release), raises(ValueError):
                require_time_budget(mutated, wall, release)
