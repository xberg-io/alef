"""Consumer stage ordering: dependency downloads never count against the extract allocation."""

import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import driver
import receipts
from assertions import raises


class ExtractReachedError(RuntimeError):
    pass


class RecordingConsumerGate:
    def __init__(self):
        self.xberg = Path("/owned/xberg")
        self.evidence = Path("/owned/evidence")
        self.calls = []

    def fetch_consumer_dependencies(self):
        driver.Gate.fetch_consumer_dependencies(self)

    def phase(self, name, _root, _target):
        self.calls.append({"name": "phase:" + name})
        return self.evidence / (name + "-cargo")

    def command(self, name, argv, cwd, budget, _timeout):
        self.calls.append({"name": name, "argv": [str(arg) for arg in argv], "cwd": cwd, "budget": budget})
        if name == "extract-current-consumer":
            raise ExtractReachedError("stop at extract")


def through_extract(gate):
    with (
        mock.patch.object(driver.Path, "read_text", return_value=""),
        mock.patch.object(driver, "safe_test_census"),
        mock.patch.object(driver.receipts, "sha", return_value="lock"),
    ):
        try:
            driver.Gate.consumer(gate, "/owned/alef-frozen")
        except ExtractReachedError:
            return
    raise AssertionError("consumer did not reach the extract stage")


class ConsumerStages(unittest.TestCase):
    def test_locked_fetch_runs_in_its_own_stage_before_extract(self):
        gate = RecordingConsumerGate()
        through_extract(gate)
        names = [row["name"] for row in gate.calls]
        assert names == ["phase:check", "fetch-current-consumer-dependencies", "extract-current-consumer"]
        fetch, extract = gate.calls[1], gate.calls[2]
        assert fetch["argv"] == ["cargo", "fetch", "--locked", "--manifest-path", "/owned/xberg/Cargo.toml"]
        assert fetch["cwd"] == gate.xberg
        assert fetch["budget"] > extract["budget"]
        assert extract["argv"][-3:] == ["extract", "--output", "/owned/evidence/ir.json"]

    def test_check_target_is_released_after_its_bridge_inputs_are_frozen(self):
        with tempfile.TemporaryDirectory() as name:
            gate = RecordingConsumerGate()
            gate.evidence = Path(name).resolve()
            gate.paths = [gate.evidence]
            target = gate.evidence / "check-target"
            out = target / "debug/build/xberg-swift-0/out"
            (out / "xberg-swift").mkdir(parents=True)
            (out / "SwiftBridgeCore.swift").write_text("core")
            (out / "xberg-swift/xberg-swift.h").write_text("header")
            (target / "debug/deps/libbulk.rlib").parent.mkdir(parents=True)
            (target / "debug/deps/libbulk.rlib").write_bytes(b"x" * 4096)
            raw = [out / "SwiftBridgeCore.swift", out / "xberg-swift/xberg-swift.h"]
            check = {"out_dir": str(out), "raw_inputs": {str(path): receipts.sha(path) for path in raw}}
            driver.release_check_target(gate.evidence, gate.paths, check, target)
            assert not target.exists()
            frozen = gate.evidence / "frozen-check"
            assert (frozen / "SwiftBridgeCore.swift").read_text() == "core"
            assert (frozen / "xberg-swift/xberg-swift.h").read_text() == "header"
            receipt = json.loads((gate.evidence / "check-target-released.json").read_text())
            assert receipt["target"] == str(target)
            assert set(receipt["before"]) == set(receipt["after"]) == {str(gate.evidence)}
            with raises(RuntimeError, "evidence-owned check phase target"):
                driver.release_check_target(gate.evidence, gate.paths, check, gate.evidence.parent / "check-target")
