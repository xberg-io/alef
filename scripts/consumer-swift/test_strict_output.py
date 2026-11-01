import unittest

from receipts import strict_output_warnings

ECHO = (
    "warning: 'swift': /Applications/Xcode_26.6.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/"
    "swift-frontend -frontend -c -primary-file /w/xberg/packages/swift/Package.swift -target arm64-apple-macosx14.0"
)


class StrictOutput(unittest.TestCase):
    def test_swiftpm_verbose_manifest_command_echo_is_not_a_warning(self):
        assert strict_output_warnings(ECHO + "\nBuilding for debugging...\n") == []

    def test_real_compiler_linker_and_build_script_warnings_are_kept(self):
        lines = [
            ECHO,
            "/w/Sources/Xberg/Xberg.swift:3:5: warning: variable 'x' was never used",
            "ld: warning: ignoring duplicate libraries: '-lc++'",
            "warning: xberg-tesseract@1.4.0: Linking with tesseract (static linking): tesseract",
            "warning: 'swift': manifest declares a deprecated platform",
        ]
        assert strict_output_warnings("\n".join(lines)) == lines[1:]


if __name__ == "__main__":
    unittest.main()
