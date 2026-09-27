//! Coverage for alef #465: `format_rust_content` must resolve its rustfmt config (and its
//! `Cargo.toml` edition) from the FILE being formatted, bounded to the project root passed in,
//! rather than from the alef process's own current working directory.
//!
//! Every test here skips (rather than fails) when `rustfmt` is not on `PATH` -- matching the
//! existing tolerance in `cli::pipeline::commands::build::frb_cfg_gates`'s own tests, which
//! `format_rust_content` is best-effort against a missing toolchain either way.

use super::normalization::format_rust_content;

/// A single expression long enough (114 columns once indented) that rustfmt's own default
/// `max_width = 100` is forced to wrap it across several lines, while a configured
/// `max_width = 150` leaves it exactly as written on one line. The gap between "wrapped" and
/// "one line" is what every test below actually asserts on -- it is a much sturdier signal than
/// comparing exact byte output, since it survives incidental rustfmt version differences in
/// how it chooses to wrap.
const LONG_EXPRESSION_SOURCE: &str = "fn f() {\n    let total = alpha_value + bravo_value + charlie_value + \
     delta_value + echo_value + foxtrot_value + golf_value;\n}\n";

/// The exact unwrapped line `LONG_EXPRESSION_SOURCE`'s body renders as when nothing wraps it.
const UNWRAPPED_LINE: &str =
    "let total = alpha_value + bravo_value + charlie_value + delta_value + echo_value + foxtrot_value + golf_value;";

const WIDE_CONFIG: &str = "max_width = 150\n";
const NARROW_CONFIG: &str = "max_width = 40\n";

fn write_rustfmt_toml(dir: &std::path::Path, content: &str) {
    std::fs::create_dir_all(dir).expect("create rustfmt.toml's directory");
    std::fs::write(dir.join("rustfmt.toml"), content).expect("write rustfmt.toml");
}

fn contains_unwrapped_line(output: &str) -> bool {
    output.lines().any(|line| line.trim() == UNWRAPPED_LINE)
}

/// THE regression this issue is about: passing `--config-path` unconditionally at a directory
/// with no config anywhere in its own upward chain is a hard error for rustfmt (verified by
/// hand: `printf 'fn x(){let y=1;}\n' | rustfmt --edition 2024 --config-path /usr` exits 1 with
/// "unable to find a config file for the given path"), not a fall-back to defaults. Before this
/// fix, `format_rust_content` took that non-success arm and returned `content` completely
/// unformatted. Sabotage: restore the unconditional `--config-path <project_root>` (or any
/// directory) in `format_rust_content` and this must fail, because the returned content would
/// again be byte-identical to the raw input.
#[test]
fn format_rust_content_falls_back_to_rustfmt_defaults_when_the_project_has_no_config() {
    if !crate::cli::pipeline::is_tool_available("rustfmt") {
        return;
    }
    let project_root = tempfile::tempdir().expect("tempdir");
    let content = "fn x(){let y=1;}\n";

    let formatted = format_rust_content(project_root.path(), &project_root.path().join("lib.rs"), content);

    assert_ne!(
        formatted.trim_end(),
        content.trim_end(),
        "a project with no rustfmt.toml anywhere must still get rustfmt's own default \
         formatting, not a hard-error fallback to the raw, unformatted input; got: {formatted:?}"
    );
    assert!(
        formatted.contains("let y = 1;"),
        "rustfmt's default formatting must have run (spaced-out assignment), got: {formatted:?}"
    );
}

/// A `rustfmt.toml` committed at the project root must be honoured regardless of what the
/// alef process's own current working directory happens to be at the time -- the entire point
/// of resolving from `project_root`/the file's own path instead of `std::env::current_dir()`.
#[test]
fn format_rust_content_honours_a_rustfmt_toml_at_the_project_root_from_an_unrelated_cwd() {
    if !crate::cli::pipeline::is_tool_available("rustfmt") {
        return;
    }
    let project_root = tempfile::tempdir().expect("tempdir");
    write_rustfmt_toml(project_root.path(), WIDE_CONFIG);
    let unrelated_cwd = tempfile::tempdir().expect("unrelated tempdir");
    let _cwd = crate::test_support::CwdGuard::enter(unrelated_cwd.path());

    let formatted = format_rust_content(
        project_root.path(),
        &project_root.path().join("src/lib.rs"),
        LONG_EXPRESSION_SOURCE,
    );

    assert!(
        contains_unwrapped_line(&formatted),
        "the project root's max_width = 150 must be honoured even though the process cwd is a \
         completely unrelated directory with no config of its own, got: {formatted:?}"
    );
}

/// A `rustfmt.toml` nearer to the file being formatted must win over one at the project root --
/// the same "nearest config wins" rule rustfmt itself documents for its own upward search,
/// applied to alef's own bounded version of that search.
#[test]
fn format_rust_content_prefers_a_nearer_rustfmt_toml_over_the_project_root() {
    if !crate::cli::pipeline::is_tool_available("rustfmt") {
        return;
    }
    let project_root = tempfile::tempdir().expect("tempdir");
    write_rustfmt_toml(project_root.path(), WIDE_CONFIG);
    let nested = project_root.path().join("pkg/sub");
    write_rustfmt_toml(&nested, NARROW_CONFIG);

    let formatted = format_rust_content(project_root.path(), &nested.join("lib.rs"), LONG_EXPRESSION_SOURCE);

    assert!(
        !contains_unwrapped_line(&formatted),
        "the nearer, narrower rustfmt.toml (max_width = 40) must win over the project root's \
         wider one (max_width = 150), got: {formatted:?}"
    );
}

/// The bounded lookup must never walk past `project_root` in the OTHER direction either: a file
/// living entirely outside the given `project_root` must never pick up a foreign tree's own
/// `rustfmt.toml`, even though that tree genuinely has one on disk.
#[test]
fn format_rust_content_does_not_escape_the_project_root() {
    if !crate::cli::pipeline::is_tool_available("rustfmt") {
        return;
    }
    let project_root = tempfile::tempdir().expect("tempdir");
    let unrelated_tree = tempfile::tempdir().expect("unrelated tempdir");
    write_rustfmt_toml(unrelated_tree.path(), WIDE_CONFIG);

    let foreign_path = unrelated_tree.path().join("lib.rs");
    let formatted = format_rust_content(project_root.path(), &foreign_path, LONG_EXPRESSION_SOURCE);

    assert!(
        !contains_unwrapped_line(&formatted),
        "a file outside `project_root` must not pick up a foreign tree's rustfmt.toml -- the \
         lookup must fall back to rustfmt's own defaults instead, got: {formatted:?}"
    );
}
