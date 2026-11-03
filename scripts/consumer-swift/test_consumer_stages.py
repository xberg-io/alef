"""Consumer stage ordering: dependency downloads never count against the extract allocation."""

import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import driver
import receipts
import swift_adapter
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

    def test_alef_post_build_ceiling_ends_inside_the_release_stage(self):
        env = driver.stage_env({"PATH": "/usr/bin"}, Path("/owned/bin"), Path("/owned/xberg"))
        ceiling = int(env["ALEF_RUN_COMMAND_TIMEOUT_SECS"])
        assert ceiling == driver.POST_BUILD_TIMEOUT
        assert ceiling > 3600
        assert ceiling + 63 < driver.RELEASE_BUILD_TIMEOUT
        assert env["PATH"] == "/owned/bin" + driver.os.pathsep + "/usr/bin"


class CompletedConsumerGate(RecordingConsumerGate):
    def __init__(self, root):
        super().__init__()
        self.xberg, self.evidence = root / "xberg", root / "evidence"
        self.paths = [root]
        self.package = self.xberg / "packages/swift"
        tests = self.package / "Tests/XbergTests/XbergTests.swift"
        tests.parent.mkdir(parents=True)
        tests.write_text("\n".join("func " + name + "() {}" for name in driver.receipts.SAFE_TESTS))
        self.evidence.mkdir()
        (self.xberg / "Cargo.lock").write_text("locked")
        self.archive = self.xberg / "actual.a"
        self.archive.write_bytes(b"!<arch>\n")
        self.binding = {
            "out_dir": str(self.evidence),
            "raw_inputs": {},
            "archives": [str(self.archive)],
            "archive_sha256": receipts.sha(self.archive),
        }

    def phase(self, name, root, target):
        target.mkdir()
        return super().phase(name, root, target)

    def metadata(self, *_args):
        return "actual-package"

    def inspect_executable(self, *_args):
        self.calls.append({"name": "inspect-executable"})

    def command(self, name, argv, cwd, budget, _timeout):
        self.calls.append({"name": name, "argv": list(argv), "cwd": cwd, "budget": budget})
        result = self.evidence / name
        result.mkdir()
        output = ""
        if name == "extract-current-consumer":
            (self.evidence / "ir.json").write_text(
                json.dumps({key: ["actual"] for key in ["types", "functions", "enums", "errors", "services"]})
            )
        if name == "canonical-safe-package-tests":
            output = (
                "\n".join(
                    "Test Case '-[XbergTests.XbergTests " + test + "]' passed" for test in driver.receipts.SAFE_TESTS
                )
                + "\nExecuted 2 tests, with 0 failures\n"
            )
        (result / "stdout.log").write_text(output)
        (result / "stderr.log").write_text("")
        return result


class SwiftPackageCommand(unittest.TestCase):
    def test_positive_consumer_command_prevents_private_sdk_autolink_and_keeps_strict_flags(self):
        with tempfile.TemporaryDirectory() as directory:
            gate = CompletedConsumerGate(Path(directory))
            with (
                mock.patch.object(driver.receipts, "binding_receipt", return_value=gate.binding),
                mock.patch.object(driver.materialized, "verify"),
                mock.patch.object(driver.native_receipts, "collect", return_value=[]),
                mock.patch.object(driver.negative, "qualify", return_value={}),
            ):
                result = driver.Gate.consumer(gate, "/owned/alef-frozen")
            assert set(result["safe_tests"]) == driver.receipts.SAFE_TESTS
            calls = [row for row in gate.calls if row["name"] == "canonical-safe-package-tests"]
            assert len(calls) == 1
            assert calls[0]["argv"] == [
                "swift",
                "test",
                "-Xswiftc",
                "-Xfrontend",
                "-Xswiftc",
                "-disable-autolink-framework",
                "-Xswiftc",
                "-Xfrontend",
                "-Xswiftc",
                "SwiftUICore",
            ]
            forwarded = swift_adapter.arguments(calls[0]["argv"][1:], "/actual/link.map")
            assert forwarded[-8:] == [
                "-Xswiftc",
                "-warnings-as-errors",
                "-Xlinker",
                "-fatal_warnings",
                "-Xlinker",
                "-map",
                "-Xlinker",
                "/actual/link.map",
            ]
