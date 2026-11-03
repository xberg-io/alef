import hashlib
import tempfile
import unittest
from pathlib import Path

import negative
import receipts
from assertions import raises

MISSING = 'Undefined symbols for architecture arm64:\n  "_callback", referenced from:\n      _owned\nld: symbol(s) not found\nclang: error: linker command failed with exit code 1\n'
FATAL = "ld: warning: ignoring duplicate libraries: '-lSystem'\nld: fatal warning(s) induced error (-fatal_warnings)\nclang: error: linker command failed with exit code 1\n"


def manifest_echo():
    source = Path(__file__).with_name("fixtures") / "swiftpm-manifest-command-echo.txt"
    data = source.read_bytes()
    assert hashlib.sha256(data).hexdigest() == "5bece194a2353f8fdb3f78754883b5e53c38af546e98c9a6a09cea42e0b6385c"
    assert len(data.splitlines()) == 1
    return data.decode()


class NegativeGate:
    real_nm = "/selected/rust/llvm-nm"
    real_swift = "/selected/swift"

    def __init__(self, root, diagnostic):
        self.xberg = root
        self.package = root / "packages/swift"
        (self.package / "Sources").mkdir(parents=True)
        (self.package / "Sources/Bridge.swift").write_text('@_cdecl("callback")\nfunc callback() {}\n')
        self.diagnostic = diagnostic
        self.calls = []

    def command(self, name, argv, cwd, budget, timeout, *, expected=0):
        self.calls.append((name, argv, cwd, budget, timeout, expected))
        result = self.xberg / name
        result.mkdir()
        output = {
            "archive-undefined": "_callback\n",
            "missing-callback-link": MISSING,
            "fatal-warning-link": self.diagnostic,
        }[name]
        if name == "fatal-warning-link":
            assert (expected, cwd) == (1, self.package)
            assert argv[-6:] == [
                "-Xlinker",
                "-fatal_warnings",
                "-Xlinker",
                "-warn_duplicate_libraries",
                "-Xlinker",
                "-lSystem",
            ]
        (result / "stdout.log").write_text(output)
        (result / "stderr.log").write_text("")
        return result


class ActualNegativeControls(unittest.TestCase):
    def test_missing_callbacks_accepts_only_the_actual_manifest_command_echo(self):
        for label in ["swift", "swift-missing-callbacks"]:
            echo = manifest_echo().replace("warning: 'swift':", "warning: '" + label + "':", 1)
            assert receipts.missing_callbacks(echo + MISSING, {"_callback"}) == ["_callback"]
            for warning in ["warning: real diagnostic\n", "warning: '" + label + "': invalid manifest\n"]:
                with self.subTest(label=label, warning=warning), raises(RuntimeError, "unexpected warning"):
                    receipts.missing_callbacks(echo + MISSING + warning, {"_callback"})

    def test_fatal_control_overrides_suppression_and_accepts_only_the_intended_link_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            gate = NegativeGate(root, manifest_echo() + FATAL)
            archive = root / "actual.a"
            archive.write_bytes(b"!<arch>\n")
            result = negative.qualify(gate, archive, gate.package)
            assert result["removed_exports"] == 1
            assert result["missing_archive_references"] == ["_callback"]
            assert [row[0] for row in gate.calls] == [
                "archive-undefined",
                "missing-callback-link",
                "fatal-warning-link",
            ]

    def test_fatal_control_rejects_unrelated_warnings_missing_fatal_and_wrong_link_failures(self):
        for diagnostic, message in [
            (FATAL + "warning: unrelated\n", "unexpected reason"),
            (FATAL.replace("System", "other"), "unexpected reason"),
            ("linker command failed\n", "unexpected reason"),
            (FATAL.replace("ld: fatal warning(s) induced error (-fatal_warnings)\n", ""), "failure not proved"),
            (FATAL + "Undefined symbols\n", "failure not proved"),
            (
                FATAL.replace("ld: fatal warning(s) induced error (-fatal_warnings)", "clang -Xlinker -fatal_warnings"),
                "failure not proved",
            ),
        ]:
            with self.subTest(diagnostic=diagnostic), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                gate = NegativeGate(root, manifest_echo() + diagnostic)
                archive = root / "actual.a"
                archive.write_bytes(b"!<arch>\n")
                with raises(RuntimeError, message):
                    negative.qualify(gate, archive, gate.package)


class SwiftNegativeCommands(unittest.TestCase):
    def test_both_real_negative_callers_prevent_private_sdk_autolink_and_keep_strict_flags(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            gate = NegativeGate(root, FATAL)
            archive = root / "actual.a"
            archive.write_bytes(b"!<arch>\n")
            result = negative.qualify(gate, archive, gate.package)
            assert result["missing_archive_references"] == ["_callback"]
            commands = {row[0]: row[1] for row in gate.calls if row[0] != "archive-undefined"}
            assert set(commands) == {"missing-callback-link", "fatal-warning-link"}
            for name, argv in commands.items():
                with self.subTest(command=name):
                    assert argv[:10] == [
                        gate.real_swift,
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
                    assert argv[10:15] == ["-v", "-Xswiftc", "-warnings-as-errors", "-Xlinker", "-fatal_warnings"]
