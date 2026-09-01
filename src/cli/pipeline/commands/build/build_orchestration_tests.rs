use super::*;
use crate::core::config::output::StringOrVec;

fn hermetic_config(toml: &str) -> ResolvedCrateConfig {
    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(toml).unwrap();
    alef_cfg.resolve().unwrap().remove(0)
}

/// Build a [`BuildCommandConfig`] override for `ResolvedCrateConfig::build_commands`, alef's own
/// `#[cfg(test)]` hermetic-test hook (0.82.0 removed `[build_commands.<lang>]` from `alef.toml`,
/// so this is the only way left to substitute a deterministic stand-in for a real npm/go/php/
/// cargo invocation). `precondition` defaults to `"true"` since every fixture in this module wants
/// the override always ready, never toolchain-gated.
fn fake_build(build: &str) -> BuildCommandConfig {
    fake_build_gated("true", build)
}

/// Same as [`fake_build`], with an explicit `precondition` instead of the always-ready default --
/// for simulating a missing toolchain (`precondition = "false"`, mirroring a `command -v` check
/// that finds nothing on `PATH`).
fn fake_build_gated(precondition: &str, build: &str) -> BuildCommandConfig {
    BuildCommandConfig {
        precondition: Some(precondition.to_string()),
        dependency_precondition: None,
        dependency_remediation: None,
        before: None,
        build: Some(StringOrVec::Single(build.to_string())),
        build_release: None,
        timeout_seconds: None,
    }
}

#[test]
fn explicit_environment_reaches_the_ffi_build_process() {
    let mut config = hermetic_config(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "environment-test-lib"
sources = ["src/lib.rs"]
"#,
    );
    config.build_commands.insert(
        Language::Ffi.to_string(),
        fake_build(r#"test "$ALEF_EXPORT_GENERATED_HEADERS" = "1""#),
    );

    build_with_environment(
        &config,
        &[Language::Ffi],
        false,
        &[("ALEF_EXPORT_GENERATED_HEADERS", "1")],
        false,
    )
    .expect("the explicit header-export environment must reach Cargo's build process");
}

/// `go` is `ffi_dependent` (`BuildDependency::Ffi`) while `php` and `node`
/// are `independent`. `php` fails; the old code's `result?` in the
/// `independent` consumption loop returned from `build()` right there,
/// before the `ffi_dependent` stage ever ran — silently dropping `go`
/// (and every other `ffi_dependent` language) with zero log output. This
/// is the "false" command-substitution incident's real blast radius.
///
/// `ffi` is included as an independent target purely so `independent`
/// already contains a `tool == "cargo" && crate_suffix == "-ffi"` entry:
/// that short-circuits `build()`'s auto FFI-crate-build step (which would
/// otherwise shell out to a real `cargo build -p <crate>-ffi` against a
/// package that doesn't exist in this synthetic config), keeping the test
/// hermetic — only `sh -c true`/`sh -c false`/`touch` ever run.
///
/// Proof that `node` and `go` were actually dispatched uses marker files
/// written by their build commands, not `tracing-test`'s `logs_contain`:
/// `node`/`go` build inside `independent`/`ffi_dependent`'s
/// `.par_iter()`, which runs on rayon's worker threads. `tracing-test`
/// scopes captured logs to a span entered via a thread-local guard on the
/// test's own thread — that guard does not propagate to rayon's pool, so
/// log lines from those closures would not carry the test's scope prefix
/// and `logs_contain` would be unreliable here regardless of whether the
/// underlying fix is correct. ~keep
#[test]
fn one_backend_failure_does_not_block_the_others() {
    let marker_dir = tempfile::tempdir().expect("failed to create temp dir for build markers");
    let marker_node = marker_dir.path().join("node.built");
    let marker_go = marker_dir.path().join("go.built");

    let mut config = hermetic_config(
        r#"
[workspace]
languages = ["php", "node", "ffi", "go"]

[[crates]]
name = "orchestration-test-lib"
sources = ["src/lib.rs"]
"#,
    );
    config
        .build_commands
        .insert(Language::Php.to_string(), fake_build("false"));
    config.build_commands.insert(
        Language::Node.to_string(),
        fake_build(&format!("touch {}", marker_node.display())),
    );
    config
        .build_commands
        .insert(Language::Ffi.to_string(), fake_build("true"));
    config.build_commands.insert(
        Language::Go.to_string(),
        fake_build(&format!("touch {}", marker_go.display())),
    );

    let result = build(
        &config,
        &[Language::Php, Language::Node, Language::Ffi, Language::Go],
        false,
        false,
    );

    assert!(result.is_err(), "php's failure must surface in the aggregate result");
    let message = format!("{:#}", result.unwrap_err());
    assert!(
        message.contains("php"),
        "aggregate error must name the failed language: {message}"
    );

    assert!(
        marker_node.exists(),
        "node (independent, ordered after php in the list) must still be attempted and succeed"
    );
    assert!(
        marker_go.exists(),
        "go (ffi_dependent) must still be attempted and succeed even though the independent \
         stage had a failure — that's the class of language that used to be silently dropped"
    );
}

/// End-to-end regression for the "never examined" gap `--strict` closes: a language whose
/// toolchain precondition fails (simulated with a `false` precondition, same as a `command -v`
/// check finding nothing on `PATH`) is skipped, not built, and by default that skip does not fail
/// the run at all -- the exit code cannot distinguish "built and passed" from "never attempted".
/// `strict = true` must turn that specific skip into a failing run, while a language that really
/// did build must still be attempted and still succeed regardless of `strict`. ~keep
#[test]
fn strict_fails_the_run_when_a_language_is_skipped_for_a_missing_toolchain() {
    let marker_dir = tempfile::tempdir().expect("tempdir");
    let node_marker = marker_dir.path().join("node.built");

    let mut config = hermetic_config(
        r#"
[workspace]
languages = ["node", "go"]

[[crates]]
name = "strict-toolchain-test-lib"
sources = ["src/lib.rs"]
"#,
    );
    config.build_commands.insert(
        Language::Node.to_string(),
        fake_build(&format!("touch {}", node_marker.display())),
    );
    config
        .build_commands
        .insert(Language::Go.to_string(), fake_build_gated("false", "true"));

    let lenient = build(&config, &[Language::Node, Language::Go], false, false);
    assert!(
        lenient.is_ok(),
        "without --strict, a toolchain-missing skip must not fail the run: {lenient:?}"
    );
    assert!(
        node_marker.exists(),
        "node has no toolchain problem and must still be built"
    );

    // A fresh marker path: the first build already ran and would otherwise make this
    // assertion trivially pass regardless of whether the second build actually re-ran node.
    let node_marker_strict = marker_dir.path().join("node.built.strict");
    let mut config = hermetic_config(
        r#"
[workspace]
languages = ["node", "go"]

[[crates]]
name = "strict-toolchain-test-lib"
sources = ["src/lib.rs"]
"#,
    );
    config.build_commands.insert(
        Language::Node.to_string(),
        fake_build(&format!("touch {}", node_marker_strict.display())),
    );
    config
        .build_commands
        .insert(Language::Go.to_string(), fake_build_gated("false", "true"));

    let strict = build(&config, &[Language::Node, Language::Go], false, true);
    let message = strict
        .map(|()| String::new())
        .unwrap_or_else(|error| format!("{error:#}"));
    assert!(
        !message.is_empty(),
        "--strict must fail the run when a language was skipped for a missing toolchain"
    );
    assert!(message.contains("--strict is set"), "{message}");
    assert!(message.contains("go"), "{message}");
    assert!(
        node_marker_strict.exists(),
        "go's toolchain skip must not stop an unrelated, ready language from being attempted"
    );
}
