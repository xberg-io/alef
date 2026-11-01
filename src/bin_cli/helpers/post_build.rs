//! Running every language's required post-build step.
//!
//! Split out of `helpers.rs` (already at this repo's 1,000-line cap) rather than left inline.

use anyhow::Result;

use crate::core::backend::CompilePolicy;

#[cfg(test)]
#[path = "post_build/swift_preservation_tests.rs"]
mod swift_preservation_tests;

/// Run every language's required post-build step, isolating one language's failure from
/// every other language's.
///
/// ~keep This used to propagate the first failure with `?` immediately, which made one
/// language's post-build break abort every later-listed language's post-build too -- even
/// though each is an independent `cargo build` (or equivalent) with no dependency on the
/// others having succeeded. `e2e::run_generators` hit the identical shape first (see its own
/// doc comment: a consumer's C backend `bail!` silently starved every later e2e backend and
/// the snippet stage for two days) and was fixed to attempt every backend regardless of
/// earlier failures, reporting all of them once every backend that could run has. This mirrors
/// that fix for `alef generate`'s post-build phase: a Swift codegen defect that fails `cargo
/// build` must not also hide whatever Kotlin/Android, Wasm, or Dart's post-build would have
/// reported for the same run.
/// Whether `language` has any post-build step configured (`generate_post_build_config(config)`
/// resolves and `post_build` is non-empty) -- the same predicate [`run_required_post_builds`]
/// uses to decide whether to run one, factored out so a caller can ask the question without
/// running anything.
fn language_has_post_build_steps(
    language: crate::core::config::Language,
    config: &crate::core::config::ResolvedCrateConfig,
) -> bool {
    crate::cli::registry::try_get_backend(language)
        .and_then(|backend| backend.generate_post_build_config(config))
        .is_some_and(|build_config| !build_config.post_build.is_empty())
}

/// Whether any of `languages` has a post-build step configured for `config`.
///
/// `run_required_post_builds` runs unconditionally on every `alef all`/`alef generate` pass --
/// unlike bindings/stubs/scaffold/docs, its steps (`RunCommand`, e.g.
/// `flutter_rust_bridge_codegen` for Dart) write directly to disk via an external tool, with no
/// `WriteReport` to say whether the bytes actually changed. That makes it invisible to any
/// "did this run change anything" signal built from write reports alone -- the format-gating
/// bug this fixes (alef #119) had exactly that shape for Dart's frb output. Since the step
/// always runs and alef cannot see whether it rewrote anything, treating its mere presence as
/// "output may have changed" is the only sound default: a false-positive costs one extra
/// (idempotent) formatting pass, a false-negative ships unformatted output forever. ~keep
pub(crate) fn languages_have_post_build_steps(
    languages: &[crate::core::config::Language],
    config: &crate::core::config::ResolvedCrateConfig,
) -> bool {
    languages
        .iter()
        .any(|&language| language_has_post_build_steps(language, config))
}

/// The subset of `languages` that have at least one post-build step configured for `config`.
///
/// `languages_have_post_build_steps` answers "does formatting need to run again at all"; this
/// answers "which package directories does it need to cover". A caller that re-scopes a
/// per-language formatting pass (`Some(&changed_languages)`, not the whole-tree `None`
/// convergence `alef all` always uses) after post-build has run needs the language list, not
/// just the bool -- post-build steps that write straight to disk (Swift's
/// `MaterializeSwiftBridge` for `RustBridgeC.h`, Dart's `flutter_rust_bridge_codegen`) leave no
/// `WriteReport` entry, so the language they touched is otherwise invisible to a
/// `changed_languages` set built purely from write reports. ~keep
pub(crate) fn languages_with_post_build_steps(
    languages: &[crate::core::config::Language],
    config: &crate::core::config::ResolvedCrateConfig,
) -> Vec<crate::core::config::Language> {
    languages
        .iter()
        .copied()
        .filter(|&language| language_has_post_build_steps(language, config))
        .collect()
}

/// The `(language, BuildConfig)` list [`run_required_post_builds`] will actually run, after
/// `compile` has had its say.
///
/// Split out of `run_required_post_builds` so the decision "which steps does this policy leave"
/// is answerable -- and testable -- without running a single external command. That matters more
/// here than for most seams: the step this drops is the one whose absence is invisible until a
/// consumer notices their Swift package never got its bridge trio, and the step it keeps
/// (`MaterializeSwiftBridge`) must survive the drop or generation stops copying the trio even on
/// the runs where a previous build did leave one in `OUT_DIR`. ~keep
fn resolve_post_build_configs(
    languages: &[crate::core::config::Language],
    config: &crate::core::config::ResolvedCrateConfig,
    compile: CompilePolicy,
) -> Vec<(crate::core::config::Language, crate::core::backend::BuildConfig)> {
    languages
        .iter()
        .filter_map(|&language| {
            let backend = crate::cli::registry::try_get_backend(language)?;
            let mut build_config = backend.generate_post_build_config(config)?;
            if compile == CompilePolicy::Skipped {
                let dropped = build_config
                    .post_build
                    .iter()
                    .filter(|step| step.invokes_rust_compiler())
                    .count();
                if dropped > 0 {
                    // Never silent: this is the one place a caller learns that the artifacts a
                    // cargo step derives (Swift's swift-bridge trio) will keep whatever content
                    // is already on disk until a real build refreshes them. ~keep
                    tracing::warn!(
                        "  [{language}] skipping {dropped} compiling post-build step(s) -- artifacts \
                         derived from them keep their current on-disk content until `alef build` runs"
                    );
                }
                build_config.post_build.retain(|step| !step.invokes_rust_compiler());
            }
            if build_config.post_build.is_empty() {
                return None;
            }
            Some((language, build_config))
        })
        .collect()
}

pub(super) fn run_required_post_builds(
    languages: &[crate::core::config::Language],
    config: &crate::core::config::ResolvedCrateConfig,
    base_dir: &std::path::Path,
    compile: CompilePolicy,
) -> Result<()> {
    let resolved = resolve_post_build_configs(languages, config, compile);
    run_resolved_post_builds(&resolved, languages.len(), config, base_dir, compile)
}

/// Run post-build steps for an already-resolved `(language, build_config)` list, aggregating
/// failures across languages instead of aborting on the first one.
///
/// Split out of [`run_required_post_builds`] so the aggregation behavior (attempt every
/// language, report every failure) can be exercised directly against a caller-supplied
/// `BuildConfig` -- in particular so tests can substitute one language's `PostBuildStep` (e.g.
/// swap a `RunCommand`'s `cmd` for one that is never installed) without reaching into the
/// registry-resolved config that `run_required_post_builds` itself uses in production. This is
/// the same seam `run_post_build` already exposes one level down (it takes an explicit
/// `&BuildConfig` rather than resolving one itself); this function extends that seam to the
/// aggregation loop above it. `total_languages` is threaded through separately from
/// `resolved.len()` because it must match `run_required_post_builds`'s original denominator (the
/// full requested language count, including languages with no post-build step at all). ~keep
fn run_resolved_post_builds(
    resolved: &[(crate::core::config::Language, crate::core::backend::BuildConfig)],
    total_languages: usize,
    config: &crate::core::config::ResolvedCrateConfig,
    base_dir: &std::path::Path,
    compile: CompilePolicy,
) -> Result<()> {
    let mut failures: Vec<String> = Vec::new();
    for (language, build_config) in resolved {
        let language = *language;
        tracing::info!("  [{language}] running post-build...");
        // This pass never invokes `cargo build` itself (see `PostBuildStep::StageFfiLibrary`'s
        // handler), so it cannot name a profile the way `alef build`'s own post-build dispatch
        // can -- ask for whichever is already on disk instead. `NoBuildRequested`, not
        // `PreferOnDisk`: the two look in the same places, but this caller is a generation
        // command that never asked for a cdylib, so a missing one is the ordinary state of an
        // unbuilt tree rather than the missed build `alef test --e2e` reports. ~keep
        match crate::cli::pipeline::run_post_build_for_generation(language, build_config, config, base_dir, compile) {
            Ok(outcome) if outcome.skipped_missing_tools.is_empty() => {
                tracing::info!("  [{language}] post-build processing complete");
            }
            // Non-fatal by design (falling back to committed generated output is the point --
            // see `run_run_command`'s doc comment), but a build that never actually ran a
            // required tool must not be reported identically to one that ran cleanly: that gap
            // is exactly what let a Dart post-build's skipped `flutter_rust_bridge_codegen`
            // masquerade as a passing run this fixes. ~keep
            Ok(outcome) => tracing::warn!(
                "  [{language}] post-build completed but skipped tool(s) not on PATH: {} -- \
                 falling back to committed generated files",
                outcome.skipped_missing_tools.join(", ")
            ),
            Err(error) => {
                tracing::warn!("[{language}] post-build failed, continuing with remaining languages: {error:#}");
                failures.push(format!("[{language}] {error:#}"));
            }
        }
    }
    if failures.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "post-build failed for {} of {} language(s): {}",
        failures.len(),
        total_languages,
        failures.join("; ")
    );
}

#[cfg(test)]
mod tests {
    use super::{
        languages_have_post_build_steps, resolve_post_build_configs, run_required_post_builds, run_resolved_post_builds,
    };
    use crate::core::backend::CompilePolicy;
    use crate::core::config::Language;

    /// A language with a real, always-runs post-build (see `run_post_build`'s
    /// `Language::Swift` arm) must be detected -- this is the "Dart frb" shape from
    /// alef #119: `languages_have_post_build_steps` is what lets the format-gate in
    /// `alef all` (`all_commands.rs`) treat such a language's presence as "output may
    /// have changed" even though the post-build step itself never reports a changed
    /// count.
    #[test]
    fn detects_a_language_with_a_configured_post_build_step() {
        assert!(languages_have_post_build_steps(
            &[Language::Swift],
            &crate::core::config::ResolvedCrateConfig::default()
        ));
    }

    /// A language with no post-build step (pyo3's `BuildConfig::post_build` is `vec![]`)
    /// must not be reported as having one -- otherwise every `alef all` run would treat
    /// every language as "may have changed", defeating the point of a targeted signal.
    #[test]
    fn reports_false_for_a_language_with_no_post_build_step() {
        assert!(!languages_have_post_build_steps(
            &[Language::Python],
            &crate::core::config::ResolvedCrateConfig::default()
        ));
    }

    /// One post-build-bearing language among several others without one is enough --
    /// mirrors a real multi-language `alef all` run where only one target (e.g. Dart)
    /// has a post-build step.
    #[test]
    fn detects_a_post_build_language_mixed_with_languages_that_have_none() {
        assert!(languages_have_post_build_steps(
            &[Language::Python, Language::Swift],
            &crate::core::config::ResolvedCrateConfig::default()
        ));
    }

    /// An empty language list has nothing to check and must not spuriously report a
    /// post-build step.
    #[test]
    fn reports_false_for_an_empty_language_list() {
        assert!(!languages_have_post_build_steps(
            &[],
            &crate::core::config::ResolvedCrateConfig::default()
        ));
    }

    /// Positive control for the two tests below: Swift's generation-time post-build genuinely
    /// contains a compiling step, so a later assertion that the step is gone is answering a
    /// real question rather than describing a config that never had one. `cargo check` is still
    /// a compile of the consumer's whole dependency graph -- cheaper than the `cargo build
    /// --release` task #541 replaced, but still minutes inside a command whose contract is
    /// "write source, do not compile". ~keep
    #[test]
    fn swifts_generation_post_build_contains_a_compiling_step_by_default() {
        let resolved = resolve_post_build_configs(
            &[Language::Swift],
            &crate::core::config::ResolvedCrateConfig::default(),
            CompilePolicy::Allowed,
        );
        let (_, build_config) = resolved
            .first()
            .expect("swift must resolve a generation-time post-build config");
        assert_eq!(
            build_config
                .post_build
                .iter()
                .filter(|step| step.invokes_rust_compiler())
                .count(),
            1,
            "swift's generate config must still carry exactly the one cargo step the \
             generation-only mode exists to drop: {:?}",
            build_config.post_build
        );
    }

    /// THE FIX, half one: a generation-only run must resolve no compiling step at all -- while
    /// still keeping `MaterializeSwiftBridge`. Dropping the materialization along with the cargo
    /// invocation would stop generation copying the swift-bridge trio even on the runs where an
    /// earlier real build already left one in `OUT_DIR`, which is a regression the consumer never
    /// asked for. ~keep
    #[test]
    fn generation_only_mode_drops_the_compiling_step_and_keeps_materialization() {
        use crate::core::backend::PostBuildStep;

        let resolved = resolve_post_build_configs(
            &[Language::Swift],
            &crate::core::config::ResolvedCrateConfig::default(),
            CompilePolicy::Skipped,
        );
        let (_, build_config) = resolved
            .first()
            .expect("swift must still resolve a post-build config once its cargo step is dropped");
        assert!(
            !build_config.post_build.iter().any(|step| step.invokes_rust_compiler()),
            "a generation-only run must invoke no compiler: {:?}",
            build_config.post_build
        );
        assert!(
            build_config
                .post_build
                .iter()
                .any(|step| matches!(step, PostBuildStep::MaterializeSwiftBridge { .. })),
            "the non-compiling materialization step must survive the drop: {:?}",
            build_config.post_build
        );
    }

    /// THE FIX, half two, observed end-to-end rather than at the config: the same call that
    /// fails below on a missing Swift cargo project must succeed when no compile was requested,
    /// because no `cargo` process is spawned at all. Nothing but the absence of that spawn can
    /// make this pass -- the temp dir has no `Cargo.toml` anywhere, so a `cargo check` reaching
    /// it can only error.
    ///
    /// Holds `SKIP_COMMANDS_LOCK` for the same reason
    /// `required_post_build_failure_is_propagated_with_language_context` does: a concurrent test
    /// setting `ALEF_SKIP_COMMANDS=cargo` would skip the invocation regardless, which would make
    /// this pass without the fix. ~keep
    #[tracing_test::traced_test]
    #[test]
    fn generation_only_mode_never_spawns_the_swift_compile() {
        let _skip_guard = crate::test_support::SkipCommandsGuard::set("");
        let directory = tempfile::tempdir().expect("temporary project");

        run_required_post_builds(
            &[Language::Swift],
            &crate::core::config::ResolvedCrateConfig::default(),
            directory.path(),
            CompilePolicy::Skipped,
        )
        .expect("a generation-only post-build pass must not attempt the swift-bridge compile");

        assert!(
            logs_contain("skipping 1 compiling post-build step"),
            "the skip must be announced, not silent -- a consumer whose swift-bridge trio stops \
             refreshing has to be able to see why"
        );
    }

    /// The missing-native-library warning, asked at the level the generation path actually runs
    /// it. Go's only post-build step is `StageFfiLibrary`, and `alef generate`/`alef all` never
    /// build the `-ffi` cdylib it stages, so on any unbuilt tree this used to emit one
    /// unavoidable "run `alef build --release`" warning per FFI-dependent language. It must now
    /// be silent here while staying a warning for `alef test --e2e` (see
    /// `ffi_stage_post_build_tests::e2e_staging_still_warns_when_the_native_library_is_missing`,
    /// the control that proves this is a gate and not a deletion). ~keep
    #[tracing_test::traced_test]
    #[test]
    fn generation_post_build_does_not_warn_about_an_unbuilt_native_library() {
        let directory = tempfile::tempdir().expect("temporary project");

        run_required_post_builds(
            &[Language::Go],
            &crate::core::config::ResolvedCrateConfig::default(),
            directory.path(),
            CompilePolicy::Allowed,
        )
        .expect("staging nothing must not fail a generation run");

        assert!(
            !logs_contain("no built FFI shared library found"),
            "a generation command never asked for a cdylib, so its absence must not be reported \
             as a missing build"
        );
        // ~keep Both halves are load-bearing, and the negative one alone is not enough. The WARN
        // and the DEBUG that replaced it share the prefix `no built FFI shared library`, so the
        // absence assertion has to name the warning's own wording (`found for target`) rather
        // than the shared prefix -- asserting the prefix fails on the fix's own DEBUG line. And
        // without the positive assertion this test would pass just as well if the staging step
        // never ran at all: a Go config that resolved no post-build step, or a miss branch never
        // reached, emits no warning either and is indistinguishable from the gate working.
        assert!(
            logs_contain("no built FFI shared library on disk"),
            "the step must still run and still report the miss -- only its severity changed"
        );
    }

    #[test]
    fn required_post_build_failure_is_propagated_with_language_context() {
        // Swift's post-build genuinely `cargo build`s the missing project below and must fail
        // for real, so this must hold `SKIP_COMMANDS_LOCK` for its whole duration -- otherwise a
        // concurrent test elsewhere in the suite that sets `ALEF_SKIP_COMMANDS=cargo` (any test
        // exercising a Swift/Dart post-build without a real toolchain) can skip this cargo
        // invocation instead of letting it fail, turning this into a false pass or, as measured,
        // a spurious failure of the OTHER test's own assertions. See `SKIP_COMMANDS_LOCK`'s doc. ~keep
        let _skip_guard = crate::test_support::SkipCommandsGuard::set("");
        let directory = tempfile::tempdir().expect("temporary project");
        let error = run_required_post_builds(
            &[Language::Swift],
            &crate::core::config::ResolvedCrateConfig::default(),
            directory.path(),
            CompilePolicy::Allowed,
        )
        .expect_err("missing Swift build project must fail");

        assert!(error.to_string().contains("swift"));
    }

    /// One language's post-build failure used to abort the loop via `?` before any later
    /// language's post-build ran at all -- so a Swift codegen defect silently hid whatever
    /// Dart's post-build would have reported for the same run, the same shape
    /// `e2e::run_generators`'s doc comment describes a consumer hitting for two days.
    ///
    /// Both languages here must fail regardless of host toolchains. Swift always fails because
    /// no build project exists in the temp dir (`cargo` is always present in this repo's own
    /// test environment). Dart's `RunCommand` step for the frb codegen tool is *not* usable
    /// as-is: whether it genuinely runs and errors, or is silently skipped because the tool
    /// isn't on `PATH` (`run_run_command`'s `NotFound` arm returns `Ok(false)`, not an error --
    /// see `PostBuildOutcome`), depends entirely on the host, and if it genuinely ran it would
    /// regenerate the bridge and defeat the "stale bridge" setup below. Forcing the skip via
    /// `ALEF_SKIP_COMMANDS` used to paper over that, but the var is process-global: this test's
    /// own `ENV_LOCK` only serialized against other holders of that *same* lock instance, not
    /// against `run_command_tests::env_lock()` in `build.rs`, which sets the identical var under
    /// a separate lock -- the exact two-locks-guarding-one-resource race `f968767b6` fixed for
    /// `frb_bridge_coverage.rs`'s equivalent test. The fix here is the same one applied there:
    /// route the RunCommand step through a command name that is never installed on any host, so
    /// `run_run_command` reports the deterministic `Ok(false)` skip without touching real
    /// process-global state at all, and no lock is needed. `run_resolved_post_builds` is the
    /// seam that makes that substitution possible -- it takes an explicit resolved
    /// `(language, BuildConfig)` list instead of resolving one from the registry internally, so
    /// Dart's registry-derived config can be cloned and have its `RunCommand` step's `cmd`
    /// swapped before the aggregation loop ever runs. A stale FRB bridge -- pre-seeded at
    /// Dart's real, registry-derived facade/bridge paths -- then makes `VerifyFrbBridgeCoverage`
    /// (a pure-Rust check with no external tool dependency) fail. Both failures must be named --
    /// proving the second language was actually attempted, not just that the error text happens
    /// to mention it. ~keep
    #[test]
    fn a_failing_language_does_not_abort_the_remaining_post_builds() {
        use crate::core::backend::PostBuildStep;
        use crate::core::config::ResolvedCrateConfig;

        // Swift's post-build genuinely `cargo build`s the missing project below and must fail
        // for real -- see `required_post_build_failure_is_propagated_with_language_context`'s
        // identical guard for why this must hold `SKIP_COMMANDS_LOCK`, not just avoid setting
        // the var itself: a concurrent test elsewhere in the suite setting
        // `ALEF_SKIP_COMMANDS=cargo` would skip this same invocation regardless. ~keep
        let _skip_guard = crate::test_support::SkipCommandsGuard::set("");
        let directory = tempfile::tempdir().expect("temporary project");
        let config = ResolvedCrateConfig::default();

        // Discover Dart's real facade/bridge paths (and full post-build steps) from its own
        // derived `BuildConfig` rather than duplicating that backend's internal path formula
        // here.
        let mut dart_build_config = crate::cli::registry::try_get_backend(Language::Dart)
            .and_then(|backend| backend.build_config_with_config(&config))
            .expect("Dart backend must produce a build config for the default crate config");
        let (facade_path, bridge_path) = dart_build_config
            .post_build
            .iter()
            .find_map(|step| match step {
                PostBuildStep::VerifyFrbBridgeCoverage {
                    facade_path,
                    bridge_path,
                    ..
                } => Some((facade_path.clone(), bridge_path.clone())),
                _ => None,
            })
            .expect("Dart's default post-build steps must include VerifyFrbBridgeCoverage");

        // Point the frb codegen `RunCommand` at a command name that is never installed on any
        // host -- see the test's doc comment for why this replaces `ALEF_SKIP_COMMANDS`.
        for step in &mut dart_build_config.post_build {
            if let PostBuildStep::RunCommand { cmd, .. } = step {
                *cmd = "alef-frb-codegen-intentionally-not-on-path-xyz789";
            }
        }

        let swift_build_config = crate::cli::registry::try_get_backend(Language::Swift)
            .and_then(|backend| backend.build_config_with_config(&config))
            .expect("Swift backend must produce a build config for the default crate config");

        // A facade that has grown a function the committed bridge never picked up -- the
        // alef #135 shape `VerifyFrbBridgeCoverage` exists to catch.
        let facade_file = directory.path().join(&facade_path);
        std::fs::create_dir_all(facade_file.parent().expect("facade path must have a parent")).unwrap();
        std::fs::write(
            &facade_file,
            "pub fn count_widgets(collection: String) -> Result<i64, String> {\n    Ok(0)\n}\n\
             pub fn record_price(id: String, price_cents: i64) -> Result<(), String> {\n    Ok(())\n}\n",
        )
        .unwrap();
        let bridge_file = directory.path().join(&bridge_path);
        std::fs::create_dir_all(bridge_file.parent().expect("bridge path must have a parent")).unwrap();
        std::fs::write(
            &bridge_file,
            "Future<int> countWidgets({required String collection}) => \
             RustLib.instance.api.crateCountWidgets(collection: collection);\n",
        )
        .unwrap();

        let resolved = [
            (Language::Swift, swift_build_config),
            (Language::Dart, dart_build_config),
        ];
        let compile = CompilePolicy::Allowed;
        let result = run_resolved_post_builds(&resolved, resolved.len(), &config, directory.path(), compile);

        let error = result.expect_err("missing Swift build project and a stale Dart bridge must both fail");
        let message = error.to_string();
        assert!(message.contains("swift"), "got: {message}");
        assert!(message.contains("dart"), "got: {message}");
        assert!(message.contains("2 of 2"), "got: {message}");
    }
}
