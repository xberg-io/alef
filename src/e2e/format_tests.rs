use super::*;

/// Whether `mix` runs, not merely resolves: a version-manager shim spawns fine then exits
/// non-zero, so a PATH-only check would take the "mix installed" branch below without mix
/// actually having reformatted anything, failing the assert that branch makes. ~keep
fn mix_is_runnable() -> bool {
    static RUNNABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *RUNNABLE.get_or_init(|| {
        std::process::Command::new("mix")
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

/// The Windows CI failure this guards is a shell-quoting defect, not a Windows one:
/// the residual step used to be `sh -c "(cd {dir} && mix format)"` with `{dir}`
/// interpolated raw. On Windows `{dir}` is `\\?\C:\...`, which POSIX `cd` rejects;
/// on any platform a space splits it into two `cd` arguments. Either way the shell
/// exits 1 before the tool runs, and 1 is not `SHELL_COMMAND_NOT_FOUND`, so an absent
/// toolchain was misreported as the formatter rejecting the code and killed the run.
/// A spaced path reproduces that on Unix, where the Windows form cannot exist. ~keep
#[test]
fn residual_step_survives_an_output_directory_whose_path_has_a_space() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("has a space").join("e2e-out");
    std::fs::create_dir_all(out.join("elixir/test")).unwrap();
    let source = "defmodule T do\n  def go, do: :ok\nend\n";
    let test_file = out.join("elixir/test/smoke_test.exs");
    std::fs::write(&test_file, source).unwrap();

    std::fs::write(
        out.join("elixir/.formatter.exs"),
        "[\n  inputs: [\"{mix,.formatter}.exs\", \"{config,lib,test}/**/*.{ex,exs}\"]\n]\n",
    )
    .unwrap();
    let files = vec![GeneratedFile {
        path: test_file.clone(),
        content: source.to_owned(),
        generated_header: false,
    }];

    let deferred = run_formatters(&files, &e2e_config_for(&out), false)
        .expect("a spaced output path must not turn an absent toolchain into a fatal failure");

    // With mix installed the step genuinely runs and there is nothing to defer, so the
    // deferral is only asserted in the branch where it is the expected outcome -- the
    // same split `default_path_formats_elixir_with_mix` uses. ~keep
    if which::which("mix").is_err() {
        assert_deferred(&deferred, "elixir", "mix format");
    }
}

/// The half of the defect that a machine WITH the toolchain installed cannot show:
/// that an absent executable is still classified as absent when the directory path
/// is not shell-safe. Under the old `sh -c` form the failed `cd` masked the tool
/// entirely, so absence and rejection became the same exit code. ~keep
#[test]
fn an_absent_program_is_classified_absent_even_in_a_spaced_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let spaced = dir.path().join("has a space");
    std::fs::create_dir_all(&spaced).unwrap();

    let failure = run_in_dir("alef-no-such-formatter", &["--check"], &spaced, "elixir")
        .expect_err("a program that does not exist cannot succeed");

    assert!(
        failure.executable_missing,
        "an absent program must be reported as missing, not as a formatter verdict: {:?}",
        failure.error
    );
}

/// The same defect on the one path `run_in_dir` cannot cover: a user `format` override is a
/// shell line, so its `{dir}` placeholder must expand to something `sh` can `cd` into. The
/// Windows form cannot exist on Unix, so the transform is asserted directly. ~keep
#[test]
fn a_verbatim_windows_path_is_rewritten_into_a_form_a_posix_shell_can_cd_into() {
    assert_eq!(
        posix_shell_path(r"\\?\C:\Users\runner\AppData\Local\Temp\e2e-out\php"),
        "C:/Users/runner/AppData/Local/Temp/e2e-out/php",
        "the extended-length prefix must be dropped and separators flipped, or every `\\` in \
         the path is eaten as a shell escape and `cd` fails before the formatter runs"
    );
    assert_eq!(
        posix_shell_path(r"\\?\UNC\server\share\e2e-out\php"),
        "//server/share/e2e-out/php",
        "a verbatim UNC path must round-trip back to its double-slash share form"
    );
    assert_eq!(
        posix_shell_path("/tmp/e2e-out/php"),
        "/tmp/e2e-out/php",
        "a path that is already POSIX must pass through untouched"
    );
}

/// GREEN: `shell_single_quote` must neutralize shell metacharacters -- a value containing
/// `;`, backticks, or `$(...)` must round-trip through a real `sh -c` invocation verbatim,
/// never as separate commands. This is the fix for `format_language`'s `{dir}` substitution,
/// which used to splice `dir` (derived from the free-form `[e2e] output` config value)
/// straight into a user override's shell text with no quoting at all.
#[test]
fn shell_single_quote_neutralizes_metacharacters() {
    for malicious in [
        "e2e-out; touch pwned_from_dir",
        "e2e-out`touch pwned_from_dir`",
        "e2e-out$(touch pwned_from_dir)",
    ] {
        let quoted = shell_single_quote(malicious);
        let output = std::process::Command::new("sh")
            .args(["-c", &format!("printf %s {quoted}")])
            .output()
            .expect("sh should run");
        assert!(
            output.status.success(),
            "quoted printf should succeed for {malicious:?}"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            malicious,
            "the whole payload must print back verbatim, not be shell-split"
        );
    }
}

/// GREEN: an embedded single quote in the value must itself be escaped correctly (the
/// close-quote/escaped-quote/reopen-quote trick), not merely tolerated by accident.
#[test]
fn shell_single_quote_escapes_embedded_quotes() {
    let malicious = "it's; touch pwned_from_dir";
    let quoted = shell_single_quote(malicious);
    let output = std::process::Command::new("sh")
        .args(["-c", &format!("printf %s {quoted}")])
        .output()
        .expect("sh should run");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), malicious);
}

/// Build an `E2eConfig` whose output directory is `out`, defaults otherwise.
fn e2e_config_for(out: &Path) -> E2eConfig {
    E2eConfig {
        output: out.to_string_lossy().into_owned(),
        ..E2eConfig::default()
    }
}

/// Assert `deferred` records exactly the absence of `step` for `language`.
///
/// A missing formatter executable is not an error under non-`--strict` mode -- it is a
/// recorded deferral (see `resolve_shell_failure`). Asserting on the record rather than
/// merely on `Ok` is the point: a run that skipped silently and a run that formatted
/// everything both look like `Ok`, and only one of them is correct. ~keep
fn assert_deferred(deferred: &[DeferredFormatting], language: &str, step: &str) {
    assert!(
        deferred
            .iter()
            .any(|entry| entry.language == language && entry.step == step && entry.reason == MISSING_TOOLCHAIN_REASON),
        "expected a deferred `{step}` for {language}, got: {deferred:?}"
    );
}

#[test]
fn formatter_directory_resolves_relative_targets_against_launch_directory() {
    let directory = tempfile::tempdir().expect("tempdir");
    let output = directory.path().join("e2e").join("python");
    std::fs::create_dir_all(&output).expect("create formatter target");

    let resolved = resolve_formatter_directory(Path::new("e2e/python"), directory.path()).expect("resolve path");

    assert!(resolved.is_absolute());
    assert_eq!(resolved, output.canonicalize().expect("canonical output"));
}

#[test]
fn formatter_directory_rejects_real_missing_targets() {
    let directory = tempfile::tempdir().expect("tempdir");
    let error = resolve_formatter_directory(Path::new("e2e/missing"), directory.path())
        .expect_err("missing formatter target must fail");

    assert!(error.to_string().contains("generated formatter path does not exist"));
}

/// A user override in `E2eConfig.format` must replace the poly pass: the
/// `{dir}` placeholder is expanded and the command is run verbatim.
#[test]
fn user_override_command_is_expanded_and_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let out = base.join("e2e-out");
    std::fs::create_dir_all(out.join("python")).unwrap();
    let sentinel = out.join("python/was_run.txt");
    let sentinel_str = sentinel.to_string_lossy().replace('\\', "/");

    let mut e2e_config = e2e_config_for(&out);
    e2e_config
        .format
        .insert("python".to_owned(), format!("touch {sentinel_str}"));

    let files = vec![GeneratedFile {
        path: out.join("python/main.py"),
        content: "x = 1\n".to_owned(),
        generated_header: false,
    }];

    assert!(!sentinel.exists());
    run_formatters(&files, &e2e_config, false).unwrap();
    assert!(
        sentinel.exists(),
        "user override command must run with {{dir}} expanded"
    );
}

/// Build a config whose output is `out` and whose format override for `lang`
/// is `command`, in the given dependency mode.
///
/// Registry mode resolves paths through `registry.output`, not `output` (see
/// `E2eConfig::effective_output`), so both are pointed at `out` to keep the two
/// modes comparing the same directory. ~keep
fn config_with_override(out: &Path, lang: &str, command: &str, dep_mode: DependencyMode) -> E2eConfig {
    let mut config = e2e_config_for(out);
    config.registry.output = out.to_string_lossy().into_owned();
    config.dep_mode = dep_mode;
    config.format.insert(lang.to_owned(), command.to_owned());
    config
}

fn one_file_in(out: &Path, lang: &str, name: &str) -> Vec<GeneratedFile> {
    vec![GeneratedFile {
        path: out.join(lang).join(name),
        content: "x = 1\n".to_owned(),
        generated_header: false,
    }]
}

/// Local mode is the correctness gate and must keep aborting on any formatter
/// failure. This is the control for the registry-mode test below: without it, a
/// passing deferral test could just mean failures are swallowed everywhere.
#[test]
fn local_mode_still_aborts_when_a_format_override_fails() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("e2e-out");
    std::fs::create_dir_all(out.join("python")).unwrap();
    let config = config_with_override(&out, "python", "exit 3", DependencyMode::Local);

    let error = run_formatters(&one_file_in(&out, "python", "main.py"), &config, false)
        .expect_err("local mode must abort on a failing formatter");

    assert!(
        error.to_string().contains("formatter for python exited"),
        "expected the formatter failure to propagate, got: {error}"
    );
}

/// The regression this fix closes: a format override that ran and rejected the code used to
/// report only its bare exit status, discarding whatever it actually printed on the way out --
/// exactly the shape of the real `(cd .../e2e/rust && cargo fmt --all)` failure this override
/// path exists to run, which surfaced with no way to tell a parse error from any other reason
/// rustfmt exits 1. The formatter's own stderr must now reach the propagated error. ~keep
#[test]
fn a_format_override_failure_quotes_the_formatters_own_stderr() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("e2e-out");
    std::fs::create_dir_all(out.join("python")).unwrap();
    let config = config_with_override(
        &out,
        "python",
        "echo alef-regression-marker-9f31 1>&2; exit 3",
        DependencyMode::Local,
    );

    let error = run_formatters(&one_file_in(&out, "python", "main.py"), &config, false)
        .expect_err("a failing override must abort the run");

    assert!(
        error.to_string().contains("alef-regression-marker-9f31"),
        "the formatter's own stderr must survive into the error, not just its exit code: {error}"
    );
}

/// The defect: a registry-mode resolver failure aborted the run, which took
/// finalisation and docs down with it. It must now be reported and survived.
#[test]
fn registry_mode_defers_a_failing_format_override_instead_of_aborting() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("e2e-out");
    std::fs::create_dir_all(out.join("python")).unwrap();
    let config = config_with_override(&out, "python", "exit 3", DependencyMode::Registry);

    let deferred = run_formatters(&one_file_in(&out, "python", "main.py"), &config, false)
        .expect("registry mode must not abort when a resolver cannot run pre-publish");

    assert_eq!(deferred.len(), 1, "expected exactly one deferred step: {deferred:?}");
    assert_eq!(deferred[0].language, "python");
    assert_eq!(deferred[0].step, "exit 3");
    assert!(
        deferred[0].reason.contains("not published yet"),
        "reason must name the unpublished pin, got: {}",
        deferred[0].reason
    );
}

/// Deferral is for failures only — a registry-mode override that succeeds must
/// still run and must report nothing, so the list cannot become a dumping ground.
#[test]
fn registry_mode_reports_nothing_when_the_override_succeeds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("e2e-out");
    std::fs::create_dir_all(out.join("python")).unwrap();
    let sentinel = out.join("python/ran.txt");
    let sentinel_str = sentinel.to_string_lossy().replace('\\', "/");
    let config = config_with_override(
        &out,
        "python",
        &format!("touch {sentinel_str}"),
        DependencyMode::Registry,
    );

    let deferred = run_formatters(&one_file_in(&out, "python", "main.py"), &config, false)
        .expect("successful override must be Ok");

    assert!(sentinel.exists(), "registry mode must still run the formatter");
    assert!(
        deferred.is_empty(),
        "a successful step must not be deferred: {deferred:?}"
    );
}

/// `go mod tidy` is dependency resolution, not formatting, and in registry mode
/// its input pins an unpublished version. It is skipped and recorded rather than
/// run-and-failed. Driven through the override map so the test needs no Go
/// toolchain: the override path proves the same defer/abort split.
#[test]
fn deferred_entry_renders_language_step_and_reason() {
    let entry = DeferredFormatting {
        language: "go".to_owned(),
        step: GO_MOD_TIDY_STEP.to_owned(),
        reason: UNPUBLISHED_VERSION_REASON.to_owned(),
    };

    let rendered = entry.to_string();

    assert!(rendered.starts_with("[go] go mod tidy — "), "got: {rendered}");
    assert!(rendered.contains("not published yet"), "got: {rendered}");
}

/// `warn_deferred`'s own prefix used to hard-code "deferred until the pinned version is
/// published" for every entry, which is only true of [`UNPUBLISHED_VERSION_REASON`]. A
/// [`MISSING_TOOLCHAIN_REASON`] entry — the shape a downstream `e2e-freshness` CI job without
/// `mix`/`go` on PATH hits on every run — got the same false claim, pointing an operator at
/// "wait for a release" when the fix is "install the toolchain". The prefix must stay
/// reason-agnostic and let each entry's own `reason` field say why. ~keep
#[test]
#[tracing_test::traced_test]
fn warn_deferred_does_not_claim_an_unpublished_version_for_a_missing_toolchain() {
    let entry = DeferredFormatting {
        language: "elixir".to_owned(),
        step: "mix format".to_owned(),
        reason: MISSING_TOOLCHAIN_REASON.to_owned(),
    };

    warn_deferred(std::slice::from_ref(&entry));

    assert!(
        !logs_contain("deferred until the pinned version is published"),
        "a missing-toolchain defer must not be blamed on an unpublished version"
    );
    assert!(
        logs_contain("executable is not installed on this machine"),
        "the actual reason must still reach the log"
    );
}

/// The registry-mode counterpart: an unpublished-version defer keeps its own reason legible
/// too, so this test is not just proving the toolchain case by omission.
#[test]
#[tracing_test::traced_test]
fn warn_deferred_reports_an_unpublished_version_reason_verbatim() {
    let entry = DeferredFormatting {
        language: "go".to_owned(),
        step: GO_MOD_TIDY_STEP.to_owned(),
        reason: UNPUBLISHED_VERSION_REASON.to_owned(),
    };

    warn_deferred(std::slice::from_ref(&entry));

    assert!(
        logs_contain("not published yet"),
        "the unpublished-version reason must still reach the log"
    );
}

/// The mixed batch is what a real release run produces, and it is where a single heading
/// cannot be right: a registry-mode run that is also missing a formatter emits both kinds at
/// once. Each kind must get its own heading, or the missing formatter is announced under the
/// publish heading and read as noise. ~keep
#[test]
#[tracing_test::traced_test]
fn a_mixed_batch_reports_each_reason_under_its_own_heading() {
    let entries = vec![
        DeferredFormatting {
            language: "php".to_owned(),
            step: "php-cs-fixer fix .".to_owned(),
            reason: MISSING_TOOLCHAIN_REASON.to_owned(),
        },
        DeferredFormatting {
            language: "go".to_owned(),
            step: GO_MOD_TIDY_STEP.to_owned(),
            reason: UNPUBLISHED_VERSION_REASON.to_owned(),
        },
    ];

    warn_deferred_for_crate("sample-crate", &entries);

    assert!(
        logs_contain("[sample-crate] 1 formatting step(s) skipped"),
        "the missing formatter needs its own heading, counting only itself"
    );
    assert!(
        logs_contain("[sample-crate] 1 dependency-resolution step(s) deferred until the pinned version is published"),
        "the publish deferral needs its own heading, counting only itself"
    );
    assert!(logs_contain("php-cs-fixer fix ."), "each entry must still be listed");
    assert!(logs_contain(GO_MOD_TIDY_STEP), "each entry must still be listed");
}

/// The default path shells out to `poly fmt --fix`. With poly installed it must
/// actually reformat the file; without it, non-strict mode must defer rather than
/// abort instead of the old behaviour of aborting regardless of `strict` -- a
/// missing default-path formatter is the same environment gap the override branch
/// already tolerated via `resolve_shell_failure`. Branches on the runner's real
/// `PATH` rather than forging one: mutating process-wide `PATH` is shared mutable
/// state across every test in this binary, the same hazard class documented on
/// `test_support::CWD_LOCK` for `set_current_dir`, and no such lock exists for env
/// vars here. ~keep
#[test]
fn default_path_formats_python_with_poly() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let out = base.join("e2e-out");
    std::fs::create_dir_all(out.join("python")).unwrap();
    let py = out.join("python/main.py");
    std::fs::write(&py, "x=1").unwrap();

    let e2e_config = e2e_config_for(&out);

    let files = vec![GeneratedFile {
        path: out.join("python/main.py"),
        content: "x=1".to_owned(),
        generated_header: false,
    }];

    if which::which("poly").is_ok() {
        run_formatters(&files, &e2e_config, false).unwrap();
        let formatted = std::fs::read_to_string(&py).unwrap();
        assert_eq!(
            formatted, "x = 1\n",
            "with poly installed, `poly fmt --fix` must reformat the e2e Python file"
        );
    } else {
        let deferred = run_formatters(&files, &e2e_config, false)
            .expect("non-strict mode must defer a missing default-path formatter, not abort");
        assert_eq!(deferred.len(), 1, "expected exactly one deferred step: {deferred:?}");
        assert_eq!(deferred[0].language, "python");
    }
}

/// Config whose python formatter names an executable that cannot exist.
fn config_with_absent_formatter(out: &Path) -> (E2eConfig, Vec<GeneratedFile>) {
    std::fs::create_dir_all(out.join("python")).unwrap();
    let py = out.join("python/main.py");
    std::fs::write(&py, "x = 1\n").unwrap();
    let mut e2e_config = e2e_config_for(out);
    e2e_config.format.insert(
        "python".to_owned(),
        "alef_formatter_that_does_not_exist {dir}".to_owned(),
    );
    let files = vec![GeneratedFile {
        path: py,
        content: "x = 1\n".to_owned(),
        generated_header: false,
    }];
    (e2e_config, files)
}

/// `--strict` keeps the original contract: an absent formatter is fatal. The
/// contract is preserved rather than deleted — it is now opt-in instead of
/// mandatory. ~keep
#[test]
fn unavailable_configured_formatter_aborts_generation_under_strict() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("e2e-out");
    let (e2e_config, files) = config_with_absent_formatter(&out);

    let error = run_formatters(&files, &e2e_config, true).expect_err("strict must fail on a missing formatter");
    assert!(
        error.to_string().contains("formatter for python exited"),
        "got: {error}"
    );
}

/// THE DEFAULT PATH, and the reason this changed: `vendor/` is gitignored in
/// consumers, so a fresh clone has no php-cs-fixer and the run died before
/// `finalize_hashes` — leaving a correctly generated but entirely unstamped tree,
/// byte-indistinguishable from a marker-stripping bug.
///
/// Surviving is only half of it. The step must be RECORDED, naming the language and
/// the command, or an absent formatter becomes a check that passed while doing
/// nothing — the exact shape this whole class of defect takes. ~keep
#[test]
fn unavailable_configured_formatter_is_recorded_and_survived_by_default() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("e2e-out");
    let (e2e_config, files) = config_with_absent_formatter(&out);

    let deferred = run_formatters(&files, &e2e_config, false).expect("a missing formatter must not abort the run");

    assert_eq!(deferred.len(), 1, "the skip must be recorded, got: {deferred:?}");
    assert_eq!(deferred[0].language, "python");
    assert!(
        deferred[0].step.contains("alef_formatter_that_does_not_exist"),
        "the record must name the command that could not run, got: {}",
        deferred[0].step
    );
    assert!(
        deferred[0].reason.contains("not installed"),
        "the record must say why, got: {}",
        deferred[0].reason
    );
}

/// The control that keeps the default honest. A formatter that RUNS and rejects the
/// code is a verdict on the generated output and still fails, with no `--strict`
/// needed — otherwise the change above would have quietly disabled the thing that
/// actually gates correctness. ~keep
#[test]
fn a_formatter_that_runs_and_fails_still_aborts_without_strict() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("e2e-out");
    std::fs::create_dir_all(out.join("python")).unwrap();
    let py = out.join("python/main.py");
    std::fs::write(&py, "x = 1\n").unwrap();
    let mut e2e_config = e2e_config_for(&out);
    e2e_config.format.insert("python".to_owned(), "exit 3".to_owned());
    let files = vec![GeneratedFile {
        path: py,
        content: "x = 1\n".to_owned(),
        generated_header: false,
    }];

    let error = run_formatters(&files, &e2e_config, false).expect_err("a formatter that ran and failed must abort");
    assert!(
        error.to_string().contains("formatter for python exited"),
        "got: {error}"
    );
}

#[test]
fn cached_paths_use_the_same_formatter_pipeline() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("e2e-out");
    std::fs::create_dir_all(out.join("python")).unwrap();
    let py = out.join("python/main.py");
    std::fs::write(&py, "x=1").unwrap();

    let e2e_config = e2e_config_for(&out);
    let deferred = run_formatters_for_cached_paths(std::slice::from_ref(&py), dir.path(), &e2e_config, false)
        .expect("a missing poly is deferred, not fatal, under non-strict mode");
    let formatted = std::fs::read_to_string(&py).unwrap();
    if which::which("poly").is_ok() {
        assert_eq!(formatted, "x = 1\n");
        assert!(deferred.is_empty(), "poly is installed, nothing to defer: {deferred:?}");
    } else {
        assert_eq!(formatted, "x=1", "without poly the cached file must be left untouched");
        assert_deferred(&deferred, "python", "poly fmt --fix");
    }
}

/// poly (and user format overrides) rewrite files via atomic rename, which
/// resets Unix permissions to 0644. run_formatters must re-assert the
/// executable bit on shebang scripts (e.g. `run_tests.php`) afterward, so the
/// generated suite stays runnable. Deterministic with or without poly: absent
/// poly leaves the file 0644, present poly may clobber it — either way the
/// post-format chmod pass restores the bit.
#[cfg(unix)]
#[test]
fn run_formatters_restores_exec_bit_on_shebang_scripts() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("e2e-out");
    std::fs::create_dir_all(out.join("php")).unwrap();
    let script = out.join("php/run_tests.php");
    let content = "#!/usr/bin/env php\n<?php\n";
    std::fs::write(&script, content).unwrap();
    // Start non-executable to prove run_formatters sets the bit.
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o644)).unwrap();

    let e2e_config = e2e_config_for(&out);
    let files = vec![GeneratedFile {
        path: script.clone(),
        content: content.to_owned(),
        generated_header: false,
    }];

    run_formatters(&files, &e2e_config, false).unwrap();

    let mode = std::fs::metadata(&script).unwrap().permissions().mode();
    assert!(
        mode & 0o111 != 0,
        "shebang script must be executable after run_formatters, got mode {mode:#o}"
    );
}

/// `.ex`/`.exs` are excluded from the poly pass, so the Elixir residual is the
/// only thing that can format them: without it the generated suite ships with
/// the emitter's unwrapped long lines. Uses a call well past the emitted
/// `.formatter.exs`'s `line_length: 140` so mix is forced to wrap it — proving
/// mix ran, not merely that the file was left alone.
#[test]
fn default_path_formats_elixir_with_mix() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("e2e-out");
    std::fs::create_dir_all(out.join("elixir/test")).unwrap();
    std::fs::write(
        out.join("elixir/.formatter.exs"),
        "[\n  inputs: [\"{mix,.formatter}.exs\", \"{config,lib,test}/**/*.{ex,exs}\"],\n  line_length: 140\n]\n",
    )
    .unwrap();
    let long_call = format!("<blockquote><p>{}</p></blockquote>", "x".repeat(160));
    let unformatted = format!("defmodule T do\n  def go do\n    {{:ok, r}} = M.convert(\"{long_call}\")\n  end\nend\n");
    let test_file = out.join("elixir/test/smoke_test.exs");
    std::fs::write(&test_file, &unformatted).unwrap();

    let e2e_config = e2e_config_for(&out);
    let files = vec![GeneratedFile {
        path: test_file.clone(),
        content: unformatted.clone(),
        generated_header: false,
    }];

    let deferred = run_formatters(&files, &e2e_config, false).expect("absent toolchains are deferred, not fatal, here");
    let formatted = std::fs::read_to_string(&test_file).unwrap();

    // poly excludes `.ex`/`.exs`, so mix alone decides whether this file is rewritten --
    // independently of whether poly itself is installed. Each absent tool is asserted on
    // its own deferral record. ~keep
    if mix_is_runnable() {
        assert_ne!(
            formatted, unformatted,
            "with mix installed, the elixir residual must reformat the over-long call"
        );
        assert!(
            formatted.contains("M.convert(\n"),
            "mix must wrap the over-long call onto its own line, got:\n{formatted}"
        );
    } else {
        assert_eq!(formatted, unformatted, "without mix the file must be left untouched");
        assert_deferred(&deferred, "elixir", "mix format");
    }
    if which::which("poly").is_err() {
        assert_deferred(&deferred, "elixir", "poly fmt --fix");
    }
}

/// A language poly does not know still runs cleanly (poly no-ops on unknown
/// files); the process must not panic or abort.
#[test]
fn unknown_language_dir_is_best_effort() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let out = base.join("e2e-out");
    std::fs::create_dir_all(out.join("cobol")).unwrap();

    let e2e_config = e2e_config_for(&out);

    let files = vec![GeneratedFile {
        path: out.join("cobol/main.cob"),
        content: "       IDENTIFICATION DIVISION.\n".to_owned(),
        generated_header: false,
    }];

    let deferred = run_formatters(&files, &e2e_config, false)
        .expect("an unknown language is best-effort whether or not poly is installed");
    if which::which("poly").is_ok() {
        assert!(deferred.is_empty(), "poly is installed, nothing to defer: {deferred:?}");
    } else {
        assert_deferred(&deferred, "cobol", "poly fmt --fix");
    }
}

/// Build an output tree with one directory per language, each with a format override that
/// appends its own name to `log`, so a completed run records the order languages ran in.
fn config_recording_order(out: &Path, log: &Path, languages: &[&str]) -> E2eConfig {
    let mut e2e_config = e2e_config_for(out);
    let log_str = log.to_string_lossy().replace('\\', "/");
    for lang in languages {
        std::fs::create_dir_all(out.join(lang)).expect("create language dir");
        e2e_config
            .format
            .insert((*lang).to_owned(), format!("echo {lang} >> {log_str}"));
    }
    e2e_config
}

fn files_for(out: &Path, languages: &[&str]) -> Vec<GeneratedFile> {
    languages
        .iter()
        .map(|lang| GeneratedFile {
            path: out.join(lang).join("main.txt"),
            content: String::new(),
            generated_header: false,
        })
        .collect()
}

/// Languages were collected into a `HashSet`, whose iteration order is randomly seeded per
/// instance, so two runs over an unchanged tree formatted in different orders. That is
/// invisible on its own, but combined with abort-on-first-failure it made the emitted bytes
/// depend on chance. Order must be stable across runs.
#[test]
fn languages_are_formatted_in_a_deterministic_order() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("e2e-out");
    let log = dir.path().join("order.log");
    let languages = ["python", "csharp", "go", "ruby", "elixir", "dart"];

    let e2e_config = config_recording_order(&out, &log, &languages);
    let files = files_for(&out, &languages);

    run_formatters(&files, &e2e_config, false).expect("first pass");
    let first = std::fs::read_to_string(&log).expect("order log");
    std::fs::remove_file(&log).expect("reset log");
    run_formatters(&files, &e2e_config, false).expect("second pass");
    let second = std::fs::read_to_string(&log).expect("order log");

    let mut expected = languages.to_vec();
    expected.sort_unstable();
    let expected = expected
        .iter()
        .map(|lang| format!("{lang}\n"))
        .collect::<Vec<_>>()
        .join("");

    assert_eq!(first, expected, "languages must be formatted in sorted order");
    assert_eq!(
        second, first,
        "two runs over an unchanged tree must format in the same order"
    );
}

/// One language's formatter failing must not decide whether the rest run. Aborting on the
/// first failure left every later language unformatted, and since the order was random, a
/// different arbitrary subset was skipped each run.
#[test]
fn a_failing_language_does_not_skip_the_others() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("e2e-out");
    let log = dir.path().join("order.log");
    let languages = ["python", "csharp", "go"];

    let mut e2e_config = config_recording_order(&out, &log, &languages);
    // `csharp` sorts first, so under abort-on-first-failure nothing else would run.
    e2e_config.format.insert("csharp".to_owned(), "exit 1".to_owned());
    let files = files_for(&out, &languages);

    let error = run_formatters(&files, &e2e_config, false).expect_err("a failing formatter must fail the run");

    let recorded = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        recorded.contains("go") && recorded.contains("python"),
        "languages after the failing one must still be formatted, recorded: {recorded:?}"
    );
    let message = format!("{error:#}");
    assert!(
        message.contains("csharp"),
        "the failing language must be named, got: {message}"
    );
    assert!(
        message.contains("1 of 3"),
        "the report must say how many of how many failed, got: {message}"
    );
}

/// Every failure is reported, not just the first, so one run surfaces the whole picture.
#[test]
fn every_failing_language_is_named() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("e2e-out");
    let log = dir.path().join("order.log");
    let languages = ["python", "csharp", "go"];

    let mut e2e_config = config_recording_order(&out, &log, &languages);
    e2e_config.format.insert("csharp".to_owned(), "exit 1".to_owned());
    e2e_config.format.insert("go".to_owned(), "exit 1".to_owned());
    let files = files_for(&out, &languages);

    let error = run_formatters(&files, &e2e_config, false).expect_err("failing formatters must fail the run");

    let message = format!("{error:#}");
    assert!(message.contains("csharp"), "got: {message}");
    assert!(message.contains("go"), "got: {message}");
    assert!(message.contains("2 of 3"), "got: {message}");
}
