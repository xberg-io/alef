import unittest

import receipts
from assertions import raises
from contracts import SAFE_TESTS
from materialized import c_tokens, swift_content


class Outcomes(unittest.TestCase):
    def test_actual_macos_two_case_format_with_zero_swift_testing_trailer(self):
        text = "\n".join(
            "Test Case '-[XbergTests.XbergTests " + name + "]' passed (0.1 seconds)." for name in SAFE_TESTS
        )
        text += "\nExecuted 2 tests, with 0 failures (0 unexpected) in 0.2 seconds\nTest run with 0 tests passed after 0.001 seconds.\n"
        assert set(receipts.tests_passed(text)) == SAFE_TESTS
        with raises(RuntimeError):
            receipts.tests_passed("Test run with 0 tests passed")
        with raises(RuntimeError):
            receipts.tests_passed(text.replace("Executed 2 tests, with 0 failures", "Executed 2 tests, with 1 failure"))

    def test_missing_callback_rejects_parser_error_unrelated_symbol_and_warning(self):
        text = 'Undefined symbols for architecture arm64:\n  "_callback", referenced from:\n      _owned\nld: symbol(s) not found\nclang: error: linker command failed with exit code 1\n'
        assert receipts.missing_callbacks(text, {"_callback"}) == ["_callback"]
        for bad in ["syntax error", text.replace("_callback", "_unrelated"), text + "warning: unexpected\n"]:
            with raises(RuntimeError):
                receipts.missing_callbacks(bad, {"_callback"})

    def test_supported_core_materializer_transforms_are_explicit(self):
        raw = "extension RustStr: Equatable {}\n    var ptr: UnsafeMutableRawPointer\n    var isOwned: Bool = true\n"
        result = swift_content(raw, True)
        assert "extension RustStr: @retroactive Equatable" in result
        assert "public var ptr:" in result
        assert "public var isOwned:" in result
        assert result.startswith("// swift-format-ignore-file\nimport RustBridgeC\n\n")
        assert result != raw

    def test_header_tokens_ignore_only_sorted_include_runs(self):
        raw = "#include <stdint.h>\n#include <stdbool.h> \ntypedef struct RustStr { uint8_t* const start; } RustStr;\n"
        formatted = "// generated\n#include <stdbool.h>\n#include <stdint.h>\ntypedef struct RustStr {\n  uint8_t *const start;\n} RustStr;\n"
        assert c_tokens(formatted) == c_tokens(raw)
        moved = "#include <stdbool.h>\ntypedef struct RustStr { uint8_t* const start; } RustStr;\n#include <stdint.h>\n"
        assert c_tokens(moved) != c_tokens(raw)
        swapped = (
            "#include <stdint.h>\n#include <stdbool.h>\ntypedef struct RustStr { const uint8_t* start; } RustStr;\n"
        )
        assert c_tokens(swapped) != c_tokens(raw)


if __name__ == "__main__":
    unittest.main()
