use super::*;

/// Real shell commands, not a stubbed runner: `true`/`false` exercise the same spawn path a
/// `command -v` or `[ -d deps ]` precondition takes, so what the test proves is what runs. ~keep
fn cfg(precondition: &str, dependency_precondition: Option<&str>) -> BuildCommandConfig {
    BuildCommandConfig {
        precondition: Some(precondition.to_string()),
        dependency_precondition: dependency_precondition.map(str::to_string),
        dependency_remediation: dependency_precondition.map(|_| "cd packages/elixir && mix deps.get".to_string()),
        before: None,
        build: None,
        build_release: None,
        timeout_seconds: None,
    }
}

/// The defect this change exists to close: the tool is installed, the checkout is not
/// prepared, and that must not be reported the same way as generated code failing to
/// compile. ~keep
#[test]
fn should_report_unfetched_dependencies_when_the_tool_is_present_but_deps_are_not() {
    let readiness = backend_readiness(Language::Elixir, &cfg("true", Some("false")));

    assert_eq!(
        readiness,
        BackendReadiness::DependenciesUnfetched {
            check: "false".to_string(),
            remediation: "cd packages/elixir && mix deps.get".to_string(),
        }
    );
}

/// The mandatory control. A backend whose preconditions all pass is dispatched, so a build
/// that then fails is still a `failure` — the fix must not be able to pass by reclassifying
/// everything it touches. ~keep
#[test]
fn should_stay_ready_when_every_precondition_passes_so_a_real_compile_failure_still_fails() {
    assert_eq!(
        backend_readiness(Language::Elixir, &cfg("true", Some("true"))),
        BackendReadiness::Ready
    );
    assert_eq!(
        backend_readiness(Language::Go, &cfg("true", None)),
        BackendReadiness::Ready
    );

    let error =
        build_outcome(&["go: undefined: Foo".to_string()], &[], &[], false).expect_err("compile failure is fatal");
    let message = error.to_string();
    assert!(message.contains("backend build failed for 1 language(s)"), "{message}");
    assert!(!message.contains("preconditions are unmet"), "{message}");
}

#[test]
fn should_report_a_missing_tool_as_a_toolchain_skip_not_as_unfetched_dependencies() {
    let readiness = backend_readiness(Language::Elixir, &cfg("false", Some("false")));

    assert_eq!(
        readiness,
        BackendReadiness::ToolchainMissing {
            precondition: "false".to_string(),
        }
    );
}

/// A machine without a language's toolchain must still be able to build the rest, so this
/// bucket alone leaves the exit status clean by default. ~keep
#[test]
fn should_exit_clean_when_the_only_thing_that_happened_was_a_toolchain_skip() {
    assert!(build_outcome(&[], &[], &["elixir".to_string()], false).is_ok());
}

/// The gap `--strict` exists to close: a toolchain skip is otherwise invisible in the exit
/// code, so a CI run that never built (or validated) a language still reports success. ~keep
#[test]
fn should_fail_the_run_under_strict_when_a_language_was_skipped_for_a_missing_toolchain() {
    let error = build_outcome(&[], &[], &["elixir".to_string(), "kotlin".to_string()], true)
        .expect_err("--strict must not exit clean over an unexamined language");
    let message = error.to_string();

    assert!(message.contains("--strict is set"), "{message}");
    assert!(message.contains("2 language(s)"), "{message}");
    assert!(message.contains("elixir"), "{message}");
    assert!(message.contains("kotlin"), "{message}");
}

/// The same skip, without `--strict`, must not gain a fatal side effect just because the
/// bucket is now threaded through `build_outcome` -- the flag is what turns it on, not its
/// mere presence. ~keep
#[test]
fn should_stay_clean_without_strict_even_when_toolchain_missing_is_non_empty() {
    assert!(build_outcome(&[], &[], &["swift".to_string()], false).is_ok());
}

/// Non-zero, but never described as a build failure: nothing was compiled, so the message
/// says what to run instead of implying the generated code is broken. Exiting 0 here is what
/// would let a downstream consumer treat a missing artifact as a built one. ~keep
#[test]
fn should_fail_the_run_for_unmet_preconditions_while_naming_them_separately_from_failures() {
    let error = build_outcome(
        &[],
        &["elixir (run `cd packages/elixir && mix deps.get`)".to_string()],
        &[],
        false,
    )
    .expect_err("unmet preconditions must not exit clean");
    let message = error.to_string();

    assert!(message.contains("1 language(s) not built"), "{message}");
    assert!(message.contains("not a compile failure"), "{message}");
    assert!(message.contains("mix deps.get"), "{message}");
    assert!(!message.contains("backend build failed"), "{message}");
}

/// Both buckets in one run stay countable on their own — the reader must be able to tell how
/// many backends were actually compiled and wrong. ~keep
#[test]
fn should_keep_failure_and_unmet_counts_separate_when_both_occur() {
    let error = build_outcome(
        &["go: undefined: Foo".to_string()],
        &["elixir (run `mix deps.get`)".to_string()],
        &[],
        false,
    )
    .expect_err("either bucket is fatal");
    let message = error.to_string();

    assert!(message.contains("backend build failed for 1 language(s)"), "{message}");
    assert!(message.contains("1 language(s) not built"), "{message}");
}
