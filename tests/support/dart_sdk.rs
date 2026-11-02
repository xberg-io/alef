use alef::snippets::validators::CapturedStreams;

pub(crate) fn forward_machine_caches(command: &mut std::process::Command) {
    for key in ["PUB_CACHE", "CARGO_HOME"] {
        if let Some(cache) = std::env::var_os(key) {
            command.env(key, cache);
        }
    }
}

#[allow(
    clippy::print_stderr,
    reason = "report the verified SDK version used by native qualification"
)]
pub(crate) fn assert_pinned_sdk(output: &CapturedStreams, scope: &str) {
    assert!(
        format!("{}{}", output.stdout, output.stderr).contains("Dart SDK version: 3.13.7 (stable)"),
        "actual {scope} Dart SDK: {}\n{}",
        output.stdout,
        output.stderr
    );
    eprintln!("{scope}_dart_sdk={}{}", output.stdout.trim(), output.stderr.trim());
}
