use alef::snippets::validators::CapturedStreams;

pub(crate) fn assert_pinned_sdk(output: &CapturedStreams, scope: &str) {
    assert!(
        format!("{}{}", output.stdout, output.stderr).contains("Dart SDK version: 3.13.7 (stable)"),
        "actual {scope} Dart SDK: {}\n{}",
        output.stdout,
        output.stderr
    );
    eprintln!("{scope}_dart_sdk={}{}", output.stdout.trim(), output.stderr.trim());
}
