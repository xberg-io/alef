import unittest

from assertions import raises
from strict_flags import ALLOWANCE, strict_flags

STRICT = ["-C", "link-arg=-Wl,-fatal_warnings", "-C", "link-arg=-Wl,-no_warn_duplicate_libraries"]


class StrictFlags(unittest.TestCase):
    def test_only_exact_source_allowance_removed_and_other_options_retained(self):
        flags, count = strict_flags({}, ["--cfg", "keep_me", "-C", ALLOWANCE, "-C", "debuginfo=2"])
        assert count == 1
        assert flags[:4] == ["--cfg", "keep_me", "-C", "debuginfo=2"]
        assert flags[4:] == STRICT

    def test_encoded_ambient_precedence_and_empty_override_preserved(self):
        flags, count = strict_flags(
            {"RUSTFLAGS": "--cfg ignored", "CARGO_ENCODED_RUSTFLAGS": "--cfg\x1factual"}, ["--cfg", "source"]
        )
        assert flags[:2] == ["--cfg", "actual"]
        assert count == 0
        empty, _ = strict_flags({"CARGO_ENCODED_RUSTFLAGS": ""}, ["--cfg", "source"])
        assert empty == STRICT

    def test_strict_link_args_are_not_appended_twice(self):
        flags, _ = strict_flags({}, STRICT)
        assert flags == STRICT

    def test_unrecognized_global_allowance_must_fail(self):
        with raises(ValueError):
            strict_flags({"RUSTFLAGS": "-Clink-args=-undefined,dynamic_lookup"}, [])


if __name__ == "__main__":
    unittest.main()
