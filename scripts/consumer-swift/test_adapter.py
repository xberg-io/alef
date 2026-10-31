import json
import os
import select
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path

import cargo_adapter


class ActualAdapter(unittest.TestCase):
    def invoke(self, source, args, stream_limit=None):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            cargo = root / "real-cargo"
            cargo.write_text("#!" + sys.executable + "\n" + source)
            cargo.chmod(0o755)
            lock = root / "Cargo.lock"
            lock.write_text("original\n")
            evidence = root / "evidence"
            evidence.mkdir()
            env = {
                **os.environ,
                "SWIFT_GATE_REAL_CARGO": str(cargo),
                "SWIFT_GATE_LOCK": str(lock),
                "SWIFT_GATE_CARGO_EVIDENCE": str(evidence),
                "PYTHONDONTWRITEBYTECODE": "1",
            }
            if stream_limit is not None:
                env["SWIFT_GATE_STREAM_LIMIT_BYTES"] = str(stream_limit)
            result = subprocess.run(
                [sys.executable, str(Path(__file__).with_name("cargo_adapter.py")), *args],
                cwd=root,
                env=env,
                capture_output=True,
                timeout=5,
                check=False,
            )
            receipts = list(evidence.glob("*/receipt.json"))
            assert len(receipts) == 1
            return result, json.loads(receipts[0].read_text())

    def test_metadata_both_streams_preserved_byte_exact(self):
        source = "import os\nos.write(1,b'\\x00\\xffmetadata\\n')\nos.write(2,b'err\\x00\\xff\\n')\n"
        result, receipt = self.invoke(source, ["metadata", "--no-deps"])
        assert result.returncode == 0
        assert result.stdout == b"\x00\xffmetadata\n"
        assert result.stderr == b"err\x00\xff\n"
        assert receipt["forwarded_argv"] == ["metadata", "--no-deps"]

    def test_real_subprocess_status_and_payload_not_swallowed(self):
        result, receipt = self.invoke(
            "import os\nos.write(1,b'payload')\nos.write(2,b'reason')\nraise SystemExit(37)\n", ["build"]
        )
        assert (result.returncode, result.stdout, result.stderr) == (37, b"payload", b"reason")
        assert receipt["exit_code"] == 37
        assert receipt["forwarded_argv"].count("--locked") == 1

    def test_actual_args_keep_release_features_and_single_lock(self):
        source = "import sys,json\nprint(json.dumps(sys.argv[1:]))\n"
        result, _receipt = self.invoke(source, ["build", "--locked", "--release", "--features", "one,two"])
        assert result.returncode == 0
        args = json.loads(result.stdout)
        assert args[:5] == ["build", "--locked", "--release", "--features", "one,two"]
        assert args.count("--locked") == 1
        assert "--message-format=json" in args
        assert "-v" in args

    def test_lock_mutation_is_explicit_failure(self):
        source = "import os,pathlib\npathlib.Path(os.environ['SWIFT_GATE_LOCK']).write_text('mutated')\n"
        result, receipt = self.invoke(source, ["check"])
        assert result.returncode == 97
        assert receipt["lock_before"] != receipt["lock_after"]

    def test_output_bound_rejects_and_retains_reason(self):
        source = "import os,time\nos.write(1,b'x'*1024)\ntime.sleep(1)\n"
        result, receipt = self.invoke(source, ["check"], stream_limit="512")
        assert result.returncode == 97
        assert receipt["failures"] == ["Cargo stream exceeded bounded evidence allowance"]

    def test_two_full_pipes_are_drained_concurrently(self):
        source = "import os\nfor i in range(64):\n os.write(1,b'o'*4096)\n os.write(2,b'e'*4096)\n"
        result, _receipt = self.invoke(source, ["metadata"])
        assert result.returncode == 0
        assert result.stdout == b"o" * 262144
        assert result.stderr == b"e" * 262144

    def test_partial_cargo_output_is_relayed_while_cargo_still_runs(self):
        read_end, write_end = os.pipe()
        child = subprocess.Popen(
            [sys.executable, "-c", "import os,time; os.write(1,b'Running rustc'); time.sleep(30)"],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        failures = []
        with tempfile.TemporaryDirectory() as name, (Path(name) / "log").open("wb") as log:
            destination = os.fdopen(write_end, "wb")
            relay = threading.Thread(
                target=cargo_adapter.relay, args=(child.stdout, destination, log, child, failures), daemon=True
            )
            relay.start()
            try:
                ready, _, _ = select.select([read_end], [], [], 5)
                assert ready == [read_end]
                assert os.read(read_end, 64) == b"Running rustc"
                assert child.poll() is None
            finally:
                child.kill()
                child.wait(timeout=2)
                relay.join(timeout=2)
                child.stdout.close()
                destination.close()
                os.close(read_end)
        assert failures == []


if __name__ == "__main__":
    unittest.main()
