import json
import tempfile
import unittest
from pathlib import Path

import rust_symbols
from assertions import raises


class SymbolGate:
    def __init__(self, root):
        self.alef = root / "old-candidate"
        self.evidence = root / "evidence"
        self.sysroot = root / "sysroot"
        self.binary = self.sysroot / "lib/rustlib/aarch64-apple-darwin/bin/llvm-nm"
        self.binary.parent.mkdir(parents=True)
        self.binary.write_bytes(b"owned tool-selection fixture")
        self.binary.chmod(0o755)
        self.alef.mkdir()
        (self.alef / "rust-toolchain.toml").write_text('[toolchain]\nchannel="1.99.1"\ncomponents=[]\n')
        self.evidence.mkdir()
        self.env = {"RUSTUP_TOOLCHAIN": "1.99.1"}
        self.identity = "release: 1.99.1\nhost: aarch64-apple-darwin\nLLVM version: 23.1.1\n"
        self.reader_version = "llvm-nm\nLLVM version 23.1.1-rust-1.99.1-stable\n"
        self.fail_version = False
        self.calls = []

    def command(self, name, argv, cwd, budget, timeout):
        self.calls.append((name, argv, cwd, budget, timeout))
        stage = self.evidence / name
        stage.mkdir()
        output = {
            "symbol-rustc-identity": self.identity,
            "symbol-rust-sysroot": str(self.sysroot),
            "symbol-nm-version": self.reader_version,
        }[name]
        (stage / "stdout.log").write_text(output)
        if self.fail_version and name == "symbol-nm-version":
            raise RuntimeError("stage failed: symbol-nm-version")
        return stage


class RustSymbolReader(unittest.TestCase):
    def test_controller_declaration_provisions_llvm_even_for_an_older_candidate(self):
        argv = rust_symbols.install_arguments()
        assert argv[:4] == ["rustup", "toolchain", "install", "1.99.1"]
        assert "llvm-tools" in [argv[i + 1] for i, value in enumerate(argv) if value == "--component"]
        source = (Path(rust_symbols.__file__).with_name("driver.py")).read_text()
        assert source.index("rust_symbols.select(gate)") < source.index("cli = gate.build_cli()")

    def test_matching_reader_records_identity_and_uses_the_exact_sysroot_binary(self):
        with tempfile.TemporaryDirectory() as directory:
            gate = SymbolGate(Path(directory))
            rust_symbols.select(gate)
            assert gate.real_nm == str(gate.binary.resolve())
            assert [row[0] for row in gate.calls] == [
                "symbol-rustc-identity",
                "symbol-rust-sysroot",
                "symbol-nm-version",
            ]
            assert gate.calls[-1][1] == [gate.binary.resolve(), "--version"]
            report = json.loads((gate.evidence / "rust-symbol-tool.json").read_text())
            assert report["llvm_version"] == "23.1.1"
            assert report["rust_release"] == "1.99.1"
            assert report["host"] == "aarch64-apple-darwin"
            assert report["selected"] == gate.real_nm
            assert len(report["executable_sha256"]) == len(report["declaration_sha256"]) == 64

    def test_older_reader_is_refused_instead_of_accepting_partial_symbol_output(self):
        with tempfile.TemporaryDirectory() as directory:
            gate = SymbolGate(Path(directory))
            gate.reader_version = "Apple LLVM version 21.0.0\n"
            with raises(RuntimeError, "LLVM reader differs from the selected Rust compiler"):
                rust_symbols.select(gate)
            assert not hasattr(gate, "real_nm")

    def test_missing_or_foreign_sysroot_reader_is_refused_without_fallback(self):
        for foreign in [False, True]:
            with self.subTest(foreign=foreign), tempfile.TemporaryDirectory() as directory:
                gate = SymbolGate(Path(directory))
                gate.binary.unlink()
                if foreign:
                    other = Path(directory) / "ambient-nm"
                    other.write_bytes(b"foreign")
                    gate.binary.symlink_to(other)
                with raises(RuntimeError, "matching Rust llvm-nm is unavailable"):
                    rust_symbols.select(gate)
                assert "symbol-nm-version" not in [row[0] for row in gate.calls]

    def test_wrong_or_incomplete_compiler_identity_is_rejected_before_reader_selection(self):
        for identity in [
            "release: 1.98.0\nhost: aarch64-apple-darwin\nLLVM version: 23.1.1\n",
            "release: 1.99.1\nhost: aarch64-apple-darwin\n",
            "release: 1.99.1\nhost: aarch64-apple-darwin\nhost: x86_64-apple-darwin\nLLVM version: 23.1.1\n",
        ]:
            with self.subTest(identity=identity), tempfile.TemporaryDirectory() as directory:
                gate = SymbolGate(Path(directory))
                gate.identity = identity
                with raises(RuntimeError, "invalid selected Rust compiler identity"):
                    rust_symbols.select(gate)
                assert [row[0] for row in gate.calls] == ["symbol-rustc-identity"]

    def test_failed_reader_version_command_cannot_be_accepted_from_its_stdout(self):
        with tempfile.TemporaryDirectory() as directory:
            gate = SymbolGate(Path(directory))
            gate.fail_version = True
            with raises(RuntimeError, "stage failed: symbol-nm-version"):
                rust_symbols.select(gate)
            assert not hasattr(gate, "real_nm")
