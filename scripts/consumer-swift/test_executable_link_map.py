"""Linker maps may contain arbitrary symbol bytes; archive provenance must remain strict."""

import tempfile
import unittest
from pathlib import Path

import driver
from assertions import raises


class RecordingExecutableGate:
    def __init__(self, root, linkmap):
        self.xberg = root / "xberg"
        self.env = {"SWIFT_GATE_LINK_MAP": str(linkmap)}
        self.loads = root / "loads"
        self.loads.mkdir()
        (self.loads / "stdout.log").write_text("/usr/lib/libSystem.B.dylib\n")
        self.calls = []

    def command(self, name, argv, cwd, budget, timeout):
        self.calls.append((name, argv, cwd, budget, timeout))
        return self.loads


def executable_fixture(root, linkmap_bytes):
    package = root / "package"
    binary = package / ".build/debug/XbergPackageTests.xctest/Contents/MacOS/XbergPackageTests"
    binary.parent.mkdir(parents=True)
    binary.write_bytes(b"hermetic executable boundary fixture")
    linkmap = root / "positive-link.map"
    if linkmap_bytes is not None:
        linkmap.write_bytes(linkmap_bytes)
    gate = RecordingExecutableGate(root, linkmap)
    archive = root / "release-target/libxberg_swift.a"
    archive.parent.mkdir()
    archive.write_bytes(b"!<arch>\n" + b"\0" * 64)
    release = {"archives": [str(archive)]}
    return gate, package, binary, release


class ExecutableLinkMap(unittest.TestCase):
    def test_archive_member_evidence_accepts_non_utf8_linker_symbols(self):
        maps = [
            b"# Object files:\n[1] /fixture/libxberg_swift.a(member.o)\n",
            b"# Symbols:\n0x1000 \xc0\xaf\xff\n[1] /fixture/libxberg_swift.a(member.o)\n",
        ]
        assert len(maps) == 2
        for linkmap_bytes in maps:
            with self.subTest(linkmap=linkmap_bytes), tempfile.TemporaryDirectory() as raw:
                gate, package, binary, release = executable_fixture(Path(raw), linkmap_bytes)
                driver.Gate.inspect_executable(gate, package, release, release["archives"][0])
                assert gate.calls == [("actual-executable-loads", ["otool", "-L", binary], gate.xberg, 0, 15)]

    def test_missing_or_wrong_archive_members_still_fail_closed(self):
        maps = [None, b"", b"libunrelated.a(member.o)", b"libxberg_swift.a\n", b"\xc0libunrelated.a(member.o)"]
        assert len(maps) == 5
        for linkmap_bytes in maps:
            with self.subTest(linkmap=linkmap_bytes), tempfile.TemporaryDirectory() as raw:
                gate, package, binary, release = executable_fixture(Path(raw), linkmap_bytes)
                with raises(RuntimeError, "link map and verbose linker argv do not prove static archive"):
                    driver.Gate.inspect_executable(gate, package, release, release["archives"][0])
                assert gate.calls == [("actual-executable-loads", ["otool", "-L", binary], gate.xberg, 0, 15)]

    def test_unique_executable_static_loads_and_exact_archive_log_remain_required(self):
        cases = [
            ("missing", "missing unique real XCTest executable", 0),
            ("duplicate", "missing unique real XCTest executable", 0),
            ("dylib", "Rust dynamic library substituted for produced archive", 1),
            ("wrong-log", "link map and verbose linker argv do not prove static archive", 1),
        ]
        assert len(cases) == 4
        for mutation, error, calls in cases:
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as raw:
                gate, package, binary, release = executable_fixture(Path(raw), b"libxberg_swift.a(member.o)")
                log = release["archives"][0]
                if mutation == "missing":
                    binary.unlink()
                elif mutation == "duplicate":
                    duplicate = package / ".build/release/XbergPackageTests.xctest/Contents/MacOS/XbergPackageTests"
                    duplicate.parent.mkdir(parents=True)
                    duplicate.write_bytes(binary.read_bytes())
                elif mutation == "dylib":
                    (gate.loads / "stdout.log").write_text("/fixture/libxberg_swift.dylib\n")
                else:
                    log = "/wrong-release-target/libxberg_swift.a"
                with raises(RuntimeError, error):
                    driver.Gate.inspect_executable(gate, package, release, log)
                assert len(gate.calls) == calls


if __name__ == "__main__":
    unittest.main()
