import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

import runtime
import session_guard
from assertions import raises
from contracts import FLOOR


class ResourceAndCleanup(unittest.TestCase):
    def test_actual_controls_entrypoint_rejects_an_empty_discovery_tree(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            source = root / "source"
            source.mkdir()
            for path in Path(__file__).parent.glob("*.py"):
                if not path.name.startswith("test_"):
                    shutil.copy2(path, source / path.name)
            evidence = root / "consumer-controls-evidence"
            evidence.mkdir()
            result = subprocess.run(
                [sys.executable, str(source / "driver.py"), "--controls"],
                env={
                    **os.environ,
                    "GITHUB_WORKSPACE": str(root),
                    "PYTHONDONTWRITEBYTECODE": "1",
                    "SWIFT_GATE_CONTROL_EVIDENCE": str(evidence),
                },
                capture_output=True,
                timeout=5,
                check=False,
            )
            assert result.returncode == 1
            assert json.loads((evidence / "census-refusal.json").read_text()) == {"expected": 51, "discovered": 0}
            assert b"control census changed or examined zero cases" in result.stderr

    def test_actual_read_only_sink_failure_is_recorded_and_owned_child_killed(self):
        with tempfile.TemporaryDirectory() as name:
            path = Path(name) / "read-only-log"
            path.write_bytes(b"original")
            child = subprocess.Popen(
                [sys.executable, "-c", "import os,time; os.write(1,b'x'*65536); time.sleep(30)"],
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
            )
            errors = []
            try:
                with path.open("rb") as sink:
                    reader = threading.Thread(
                        target=runtime.capture_stream, args=(child.stdout, sink, child, errors), daemon=True
                    )
                    reader.start()
                    reader.join(timeout=3)
                    if reader.is_alive():
                        child.kill()
                        reader.join(timeout=2)
                    assert not reader.is_alive()
                    assert errors == ["stage stream capture failed: UnsupportedOperation"]
                    assert child.wait(timeout=2) < 0
                assert path.read_bytes() == b"original"
            finally:
                if child.poll() is None:
                    child.kill()
                child.wait(timeout=2)
                child.stdout.close()

    def test_real_cache_paths_are_counted_and_empty_inventory_rejected(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            cargo, target = root / "cargo", root / "target"
            cargo.mkdir()
            target.mkdir()
            receipt = runtime.free_receipt([cargo, target])
            assert set(receipt) == {str(cargo.resolve()), str(target.resolve())}
            assert all(row["free"] > 0 for row in receipt.values())
            with raises(ValueError):
                runtime.free_receipt([])

    def test_insufficient_actual_floor_prevents_child_spawn(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            with (
                patch.dict(os.environ, {"SWIFT_GATE_GUARDED": "1"}),
                patch.object(runtime, "free_receipt", return_value={str(root): {"free": FLOOR - 1}}),
                patch.object(runtime.subprocess, "Popen") as spawn,
            ):
                with raises(RuntimeError):
                    runtime.run(
                        "refused", ["never-run"], root, env={}, evidence=root, paths=[root], budget=1, timeout=1
                    )
                spawn.assert_not_called()
            assert (root / "refused/refusal.json").is_file()

    def test_reviewed_guard_reaps_descendant_that_changes_process_group(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            script = root / "owned.py"
            script.write_text(
                "import os,time\npid=os.fork()\nif pid==0:\n os.setpgrp()\n time.sleep(30)\nelse:\n time.sleep(30)\n"
            )
            events = []
            code = session_guard.supervise(
                [sys.executable, str(script)], root, events.append, floor=1, wall=2, sampler=lambda: 2, interval=0.02
            )
            assert code == 87
            spawns = [row for row in events if row["event"] == "spawn"]
            assert len(spawns) == 1
            assert session_guard.session_members(spawns[0]["sid"]) == []
            start = [row for row in events if row["event"] == "cleanup_start"]
            assert len(start) == 1
            assert len(start[0]["members"]) >= 2
            assert len({row["pgid"] for row in start[0]["members"]}) >= 2


if __name__ == "__main__":
    unittest.main()
