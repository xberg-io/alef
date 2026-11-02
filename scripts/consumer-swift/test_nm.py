import hashlib
import tempfile
import unittest
from pathlib import Path

import negative
from assertions import raises
from negative import undefined_symbols


class DarwinUndefinedSymbols(unittest.TestCase):
    def fixture(self, name, expected_sha):
        content = (Path(__file__).with_name("fixtures") / name).read_bytes()
        assert hashlib.sha256(content).hexdigest() == expected_sha
        return content.decode()

    def test_actual_preserved_nm_u_logs_have_bare_symbols_and_real_callbacks(self):
        control = self.fixture(
            "darwin-nm-u-control.txt", "de209c950740a6a8e08a5c45ffc3bd51d226422b78093c88de6417ab1a40b3cb"
        )
        assert undefined_symbols(control) == {"___stderrp", "_exit", "_fprintf", "_puts"}
        swift = self.fixture(
            "darwin-swift-dylib-nm-u.txt", "d343ead5e6249afaef6476181a928b2a9e425b34d7dca47bb99eacb43e91f258"
        )
        actual = undefined_symbols(swift)
        assert len(actual) > 2
        assert {name for name in actual if "__swift_bridge__$" in name} == {
            "___swift_bridge__$CounterBox$_free",
            "___swift_bridge__$CounterBox$evaluate",
        }

    def test_actual_rust_llvm_archive_symbols_preserve_callbacks_and_macho_names(self):
        text = self.fixture(
            "rust-llvm23-darwin-undefined.txt", "a92a14fcf9c8057c1e61df6c6b22d569b036ac95b1b73cb84d5b683dea2767a1"
        )
        actual = undefined_symbols(text)
        assert len(actual) == 109
        assert sum(name.startswith("section$") for name in actual) == 4
        assert sum(name.startswith("_objc_msgSend$") for name in actual) == 35
        assert sum("__swift_bridge__$" in name for name in actual) == 70

    def test_archive_inspection_requires_selected_tool_and_explicit_bare_undefined_output(self):
        class CapturedError(RuntimeError):
            pass

        class Gate:
            real_nm = "/pinned/rust/lib/rustlib/aarch64-apple-darwin/bin/llvm-nm"
            xberg = Path("/owned/consumer")

            def command(self, name, argv, cwd, budget, timeout):
                assert (cwd, budget, timeout) == (self.xberg, 0, 90)
                assert name == "archive-undefined"
                assert argv == [self.real_nm, "--undefined-only", "--just-symbol-name", str(archive)]
                raise CapturedError("selected nm invocation recorded")

        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "archive.a"
            archive.write_bytes(b"!<arch>\n")
            with raises(CapturedError, "selected nm invocation recorded"):
                negative.qualify(Gate(), archive, Path(directory))

    def test_archive_member_headers_and_duplicate_references_preserve_symbol_set(self):
        text = "\ncallback_native_probe.callback_native_probe.85140ff8c65c031-cgu.0.rcgu.o:\n_callback\n\nlibprobe.a(next.rcgu.o):\n_callback\n_other\n"
        assert undefined_symbols(text) == {"_callback", "_other"}

    def test_empty_or_header_only_output_cannot_pass_as_zero_work(self):
        for text in ["", "\n\n", "member.o:\n\n", "libprobe.a(member.o):\n"]:
            with self.subTest(text=text), raises(ValueError):
                undefined_symbols(text)

    def test_typed_u_defined_symbols_and_malformed_rows_are_rejected(self):
        for text in [
            "U _callback\n",
            "                 U _callback\n",
            "0000000000000000 T _callback\n",
            "_callback\nunrecognized:\n",
            "section$start$__DATA$\n",
            "section$middle$__DATA$_field\n",
            "_objc_msgSend$bad selector:\n",
            "_objc_msgSend$:\n",
            "_objc_msgSend$foo::\n",
            "_callback unexpected\n",
            "_callback\nerror: failed\n",
        ]:
            with self.subTest(text=text), raises(ValueError):
                undefined_symbols(text)


if __name__ == "__main__":
    unittest.main()
