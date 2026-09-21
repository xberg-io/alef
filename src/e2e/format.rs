//! Post-generation formatter support for e2e test projects.
//!
//! Formatting is delegated to the `poly` (polylint) CLI as a system dependency —
//! the same tool the main generate pipeline uses (see `cli::pipeline::format`).
//! For each language directory that had files generated, `run_formatters` runs a
//! single `poly fmt --fix` pass, which formats every language poly supports
//! (Python via ruff, JS/TS/JSON via oxc, Rust via rustfmt, Go via gofmt, …).
//! Languages are formatted in sorted order, and every language is attempted
//! before failures are reported, so which languages get formatted never depends
//! on iteration order or on whether an earlier one failed. A formatter that RUNS
//! and rejects the code still fails generation, naming every language that failed;
//! a formatter whose executable is absent is recorded and survived instead, unless
//! `--strict` asks for the old behaviour (see [`ShellFailure`]).
//!
//! Two escape hatches remain:
//! * a per-language `E2eConfig.format` override (`sh -c`, with `{dir}` expanded)
//!   replaces the poly pass for that language;
//! * a residual `go mod tidy` runs for Go directories — it is not formatting but
//!   is required to populate `go.sum` from `go.mod` so the e2e Go suite builds.
//!
//! That second escape hatch is why this stage distinguishes *formatting* from
//! *dependency resolution*. Under [`DependencyMode::Registry`] the generated
//! manifests pin the very version the current run produces, so any step that
//! resolves them against a registry cannot succeed until that version is
//! published — see [`DeferredFormatting`]. ~keep

use crate::core::backend::GeneratedFile;
use crate::core::config::e2e::DependencyMode;
use crate::e2e::config::E2eConfig;
use anyhow::Context as _;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::warn;

/// A dependency-resolving step that could not run because the version the
/// generated manifests pin is not published yet.
///
/// Registry mode exists to exercise *published* artifacts, so during the run that
/// produces those artifacts the pinned version does not exist in any registry.
/// Resolving it is therefore a post-publish activity, and letting it abort the
/// generation that must happen *first* makes the release unreachable: publishing
/// needs a complete run, and a complete run needs the publish. Deferring breaks
/// that cycle without skipping any actual formatting. ~keep
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferredFormatting {
    /// e2e language directory the step belonged to.
    pub language: String,
    /// The command, or built-in step name, that was deferred.
    pub step: String,
    /// Why it could not run now.
    pub reason: String,
}

impl DeferredFormatting {
    /// Whether this entry records an absent executable rather than an unpublished version.
    ///
    /// The two whys demand opposite operator actions — "install the toolchain" versus "wait
    /// for the release" — so every reporter has to tell them apart. [`MISSING_TOOLCHAIN_REASON`]
    /// is written verbatim at both sites that record an absent executable, and is the only
    /// reason that is, which is what makes the comparison the classification rather than a
    /// heuristic over free text. ~keep
    pub fn is_missing_toolchain(&self) -> bool {
        self.reason == MISSING_TOOLCHAIN_REASON
    }
}

impl std::fmt::Display for DeferredFormatting {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "[{}] {} — {}", self.language, self.step, self.reason)
    }
}

/// Recorded when a resolver is skipped outright because its input cannot exist yet.
const UNPUBLISHED_VERSION_REASON: &str = "registry-mode manifests pin the version this run produces, which is not \
                                          published yet; re-run after publishing";

/// Run per-language formatters for all languages that had files generated.
///
/// E2e files are written to `{output}/{lang}/...`, so the language is the first
/// path component after the output prefix. For each language directory: a user
/// `E2eConfig.format[lang]` override runs as a shell command (`{dir}` expanded);
/// otherwise poly formats the directory in-process. Languages run in sorted
/// order and each is attempted regardless of earlier failures; the run then
/// fails with every failing language named.
///
/// Actual formatting (poly, `mix format`) aborts generation in every mode — it
/// needs no registry and so has no pre-release excuse. Only *dependency
/// resolution* is treated differently: under [`DependencyMode::Registry`] a
/// resolving step is recorded as a [`DeferredFormatting`] instead of failing the
/// run, because the version it would resolve is the one this run is producing.
/// [`DependencyMode::Local`] behaviour is unchanged and always yields an empty
/// list. ~keep
pub fn run_formatters(
    files: &[GeneratedFile],
    e2e_config: &E2eConfig,
    strict: bool,
) -> anyhow::Result<Vec<DeferredFormatting>> {
    let defer_resolution = e2e_config.dep_mode == DependencyMode::Registry;
    let mut deferred = Vec::new();
    let output_prefix = Path::new(e2e_config.effective_output());
    let current_dir = std::env::current_dir().context("failed to resolve formatter working directory")?;
    // Sorted, not `HashSet` iteration order. `HashSet` is randomly seeded per process, so the
    // order languages were formatted in varied run to run; combined with the abort-on-first-
    // failure below, a single failing language left a *different* arbitrary subset of the others
    // unformatted each time, and regenerating an unchanged tree produced different bytes. ~keep
    let mut languages: Vec<String> = files
        .iter()
        .filter_map(|f| {
            let remainder = f.path.strip_prefix(output_prefix).ok()?;
            let first = remainder.components().next()?;
            Some(first.as_os_str().to_string_lossy().into_owned())
        })
        .collect::<HashSet<String>>()
        .into_iter()
        .collect();
    languages.sort();

    // Format every language before reporting rather than aborting on the first failure. Whether
    // one language's formatter fails must not decide whether the others run at all -- that made
    // the emitted tree depend on ordering. The run still fails, naming every failure. ~keep
    let mut failures: Vec<String> = Vec::new();
    for lang in &languages {
        if let Err(error) = format_language(lang, e2e_config, &current_dir, defer_resolution, strict, &mut deferred) {
            failures.push(format!("{lang}: {error:#}"));
        }
    }

    // poly (and user format overrides) rewrite files via atomic rename, which
    // resets Unix permissions to 0644 — clobbering the executable bit the scaffold
    // writer set on shebang scripts (e.g. `run_tests.php`). Re-assert it so shebang
    // e2e scripts stay executable after formatting. Paths are relative to the
    // process cwd (the repo root), matching where the writer/poly operate.
    for file in files {
        if file.content.starts_with("#!")
            && let Err(e) = crate::cli::pipeline::apply_shebang_chmod(&file.path, &file.content)
        {
            warn!("failed to restore exec bit on {}: {e}", file.path.display());
        }
    }
    if !failures.is_empty() {
        anyhow::bail!(
            "formatting failed for {} of {} language(s): {}",
            failures.len(),
            languages.len(),
            failures.join("; ")
        );
    }
    Ok(deferred)
}

/// Run the configured formatter for one generated language directory.
///
/// Split out of [`run_formatters`] so a failure can be collected per language instead of
/// aborting the whole pass -- see the ordering note there. ~keep
fn format_language(
    lang: &str,
    e2e_config: &E2eConfig,
    current_dir: &Path,
    defer_resolution: bool,
    strict: bool,
    deferred: &mut Vec<DeferredFormatting>,
) -> anyhow::Result<()> {
    let configured_dir = PathBuf::from(format!("{}/{}", e2e_config.effective_output(), lang));
    let dir_path = resolve_formatter_directory(&configured_dir, current_dir)?;
    let dir = shell_directory(&dir_path);

    // User override takes precedence and replaces the poly pass entirely. Its
    // contents are opaque to us, so registry-mode failures are deferred.
    //
    // `dir` is substituted, not written by the user -- it is derived from
    // `e2e_config.effective_output()`, a free-form `[e2e] output` path like any other
    // `alef.toml` value. The override's shell TEXT is legitimately opaque and stays shell (a
    // user wrote it, meaning it to run as shell), but the DATA alef substitutes into it is not
    // the user's to have made shell-safe -- `{dir}` is shell-quoted here so a path containing
    // `;`, backticks, or `$(...)` cannot be reinterpreted. Every documented/tested `{dir}`
    // usage in this codebase places the placeholder bareword (`cd {dir} && ...`), where
    // single-quoting is a no-op for an ordinary path and neutralizes a malicious one; an
    // override that already wraps `{dir}` in its own quotes would double-quote (a real
    // behavior change with no known/tested case of that usage today). ~keep
    if let Some(custom) = e2e_config.format.get(lang) {
        let cmd = custom.replace("{dir}", &shell_single_quote(&dir));
        tracing::debug!("Formatting {lang}: {cmd}");
        // An override IS the formatting step for `lang`, not a dependency resolver -- the
        // same contract `resolve_shell_failure` already enforces for `mix format` and (at
        // its own call site) `go mod tidy`. `dep_mode` is therefore irrelevant here: only a
        // missing executable is ever survived, in every mode. A version-of-this-line that
        // deferred ANY override failure under registry mode used to treat a formatter that
        // ran and rejected the code exactly like a resolver that cannot run pre-publish,
        // which is how a real `dart = "cd {dir} && dart format ."` override's rejection went
        // unreported (#408): the run "succeeded" while `test_apps/dart` stayed unformatted,
        // and `alef verify` had already hashed that unformatted output. ~keep
        return match run_shell(&cmd, lang) {
            Ok(()) => Ok(()),
            Err(failure) => resolve_shell_failure(failure, lang, &cmd, strict, deferred),
        };
    }

    // Default: shell out to `poly fmt --fix` over the directory. poly walks up
    // from `dir_path` for a `poly.toml` (falling back to poly's zero-config
    // defaults when none is found).
    //
    // A missing `poly` executable is the same "environment gap" the override branch
    // above already treats leniently under non-`--strict` mode -- checked explicitly
    // here (rather than routed through `poly_format_strict`'s own bail, which cannot
    // tell a missing executable from poly running and rejecting the code) so this
    // branch honors `strict` exactly the way the override branch and this module's own
    // doc comment promise, instead of always failing hard regardless of `strict`. ~keep
    tracing::debug!("Formatting {lang} with poly: {dir}");
    if !crate::cli::pipeline::is_tool_available("poly") {
        if strict {
            anyhow::bail!("poly not found on PATH; generated output cannot be formatted");
        }
        warn!("{lang}: poly fmt skipped — executable not found; continuing so the run reaches finalisation");
        deferred.push(DeferredFormatting {
            language: lang.to_owned(),
            step: "poly fmt --fix".to_owned(),
            reason: MISSING_TOOLCHAIN_REASON.to_owned(),
        });
    } else {
        crate::cli::pipeline::poly_format_strict(std::slice::from_ref(&dir_path), &dir_path)?;
    }

    // Residual: `go mod tidy` populates `go.sum` from `go.mod` (poly cannot —
    // it is dependency resolution, not formatting) so the Go suite builds.
    if lang == "go" {
        if defer_resolution {
            warn!("skipping `go mod tidy` for {lang}: {UNPUBLISHED_VERSION_REASON}");
            deferred.push(DeferredFormatting {
                language: lang.to_owned(),
                step: GO_MOD_TIDY_STEP.to_owned(),
                reason: UNPUBLISHED_VERSION_REASON.to_owned(),
            });
        } else {
            run_go_mod_tidy(&dir_path, lang, strict, deferred)?;
        }
    }

    // Residual: `mix format` is the SOLE formatter for `.ex`/`.exs` — the poly
    // pass above excludes them (see `POLY_ELIXIR_EXCLUDE_GLOBS`), so without
    // this the generated Elixir suite is never formatted at all and ships with
    // the emitter's unwrapped long lines.
    if lang == "elixir" {
        run_mix_format(&dir_path, lang, strict, deferred)?;
    }
    Ok(())
}

fn resolve_formatter_directory(path: &Path, current_dir: &Path) -> anyhow::Result<PathBuf> {
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        current_dir.join(path)
    };
    absolute_path
        .canonicalize()
        .with_context(|| format!("generated formatter path does not exist: {}", absolute_path.display()))
}

/// Format files restored from the generation-stage cache.
///
/// Cached paths are absolute while e2e generation records paths relative to the
/// consumer root. Rebuilding the lightweight file descriptors keeps cache hits on
/// the same formatter path as fresh generation, including custom format commands
/// and executable-bit restoration. ~keep
pub fn run_formatters_for_cached_paths(
    paths: &[PathBuf],
    base_dir: &Path,
    e2e_config: &E2eConfig,
    strict: bool,
) -> anyhow::Result<Vec<DeferredFormatting>> {
    let output_is_absolute = Path::new(e2e_config.effective_output()).is_absolute();
    let files: Vec<GeneratedFile> = paths
        .iter()
        .filter_map(|path| {
            let formatter_path = if output_is_absolute {
                path.clone()
            } else {
                path.strip_prefix(base_dir).ok()?.to_path_buf()
            };
            let content = std::fs::read_to_string(path).unwrap_or_default();
            Some(GeneratedFile {
                path: formatter_path,
                content,
                generated_header: true,
            })
        })
        .collect();
    run_formatters(&files, e2e_config, strict)
}

/// The status a POSIX shell exits with when the command it was asked to run does not
/// exist. ~keep
const SHELL_COMMAND_NOT_FOUND: i32 = 127;

/// Render a canonicalized directory for interpolation into the `{dir}` placeholder of a
/// user `format` override, which [`run_shell`] hands to `sh -c`.
///
/// [`run_in_dir`] escapes this problem entirely by never going through a shell; a user
/// override *is* a shell line, so the path itself has to be shell-usable. On Windows
/// `canonicalize` returns the extended-length form `\\?\C:\...`, and a POSIX shell reads
/// every `\` as an escape -- `cd \\?\C:\Users\...` becomes `cd \?C:Users...`, which fails
/// before the formatter is ever reached. The shell then exits 1, and 1 is not
/// [`SHELL_COMMAND_NOT_FOUND`], so an absent formatter arrived as "the formatter ran and
/// rejected the code" and killed the run. ~keep
fn shell_directory(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        posix_shell_path(&text)
    } else {
        text.into_owned()
    }
}

/// Wrap `value` in single quotes for safe interpolation into a POSIX shell command line,
/// escaping any embedded `'` via the standard close-quote/escaped-quote/reopen-quote trick.
/// Mirrors `cli::pipeline::helpers::inline_env_in_shell_cmd`'s value-side escaping, applied
/// here to alef's own `{dir}` substitution instead of to an env var value.
fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Strip Windows' extended-length prefix and switch to forward slashes, the drive-letter
/// form `sh` accepts. Kept free of `cfg` so it is unit-testable on every platform.
fn posix_shell_path(text: &str) -> String {
    let stripped = match text.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => text.strip_prefix(r"\\?\").unwrap_or(text).to_owned(),
    };
    stripped.replace('\\', "/")
}

/// A shell-invoked formatter that did not succeed, and whether the executable was
/// even there.
///
/// `sh -c` starts fine whether or not the formatter exists, so both cases arrive as a
/// non-zero exit and the `Err` arm of `Command::status()` is never reached. Only a
/// formatter that RAN is a verdict on the generated code; a missing one is an
/// environment gap. Conflating them made the pipeline fresh-clone hostile — `vendor/`
/// is gitignored in consumers, so a fresh clone has no php-cs-fixer and the run died
/// before `finalize_hashes`, leaving a correctly generated but entirely unstamped tree:
/// invisible to `alef verify`, failing the consumer's poly gate, and
/// byte-indistinguishable from a marker-stripping bug. ~keep
struct ShellFailure {
    executable_missing: bool,
    error: anyhow::Error,
}

/// Runs `cmd` under `sh -c`, capturing its output instead of inheriting the parent's stdio.
///
/// A formatter that ran and rejected the code used to report only its bare exit status --
/// `formatter for rust exited with exit status: 1: (cd .../e2e/rust && cargo fmt --all)` said
/// nothing about what rustfmt actually objected to, so the only way to find out was to
/// reconstruct and re-run the command by hand. Capturing costs the live-progress view
/// `Stdio::inherit()` gave for free, but formatter passes run in seconds, not the 30-minute
/// builds `run_run_command` (`cli::pipeline::commands::build`) has to stay live for. ~keep
fn run_shell(cmd: &str, lang: &str) -> Result<(), ShellFailure> {
    match std::process::Command::new("sh").args(["-c", cmd]).output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(ShellFailure {
            executable_missing: output.status.code() == Some(SHELL_COMMAND_NOT_FOUND),
            error: anyhow::anyhow!(
                "formatter for {lang} exited with {}: {cmd}\n{}",
                output.status,
                format_shell_output(&output)
            ),
        }),
        Err(error) => Err(ShellFailure {
            executable_missing: error.kind() == std::io::ErrorKind::NotFound,
            error: anyhow::Error::new(error).context(format!("failed to run formatter for {lang}: {cmd}")),
        }),
    }
}

/// Renders a failed shell formatter's captured stdout/stderr for [`ShellFailure::error`].
///
/// Both streams, not just stderr: several formatters this override wraps (php-cs-fixer, ruff
/// under some configs) write their real diagnostic to stdout, so surfacing only stderr would
/// still leave some of these failures unexplained.
fn format_shell_output(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = stdout.trim();
    let stderr = stderr.trim();
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("--- stderr ---\n{stderr}\n--- stdout ---\n{stdout}"),
        (false, true) => format!("--- stdout ---\n{stdout}"),
        (true, false) => format!("--- stderr ---\n{stderr}"),
        (true, true) => "<no output>".to_string(),
    }
}

/// Reason recorded when a formatter's executable is not installed on this machine.
const MISSING_TOOLCHAIN_REASON: &str = "the formatter's executable is not installed on this machine; generation \
                                        continued so the run still reaches finalisation. Install the toolchain, or \
                                        re-run with --strict to make this fatal";

/// Decide what an unsuccessful shell formatter means for the run.
///
/// A formatter that ran and rejected the code always fails the run — that is what
/// actually gates correctness, and it is unchanged. A formatter that is simply absent
/// is recorded and survived, unless `strict` asks for the old behaviour back.
///
/// Recording rather than skipping quietly is the whole point: an absent formatter that
/// nothing reports is the same shape as a check that passes while examining nothing. ~keep
fn resolve_shell_failure(
    failure: ShellFailure,
    lang: &str,
    step: &str,
    strict: bool,
    deferred: &mut Vec<DeferredFormatting>,
) -> anyhow::Result<()> {
    if !failure.executable_missing || strict {
        return Err(failure.error);
    }
    warn!("{lang}: `{step}` skipped — executable not found; continuing so the run reaches finalisation");
    deferred.push(DeferredFormatting {
        language: lang.to_owned(),
        step: step.to_owned(),
        reason: MISSING_TOOLCHAIN_REASON.to_owned(),
    });
    Ok(())
}

/// Step name recorded when `go mod tidy` is deferred.
const GO_MOD_TIDY_STEP: &str = "go mod tidy";

/// Log any deferred resolution steps, for standalone stage commands — which have no
/// later pipeline phase to report from the way `alef all` does.
pub fn warn_deferred(deferred: &[DeferredFormatting]) {
    report_deferred(None, deferred);
}

/// [`warn_deferred`] for a pipeline that reports per crate (`alef all`).
///
/// Shares one implementation with the standalone reporter deliberately: the two used to
/// classify the same `DeferredFormatting` list differently, and only one of them was ever
/// fixed. ~keep
pub fn warn_deferred_for_crate(crate_name: &str, deferred: &[DeferredFormatting]) {
    report_deferred(Some(crate_name), deferred);
}

/// Report deferred steps grouped by WHY they were deferred.
///
/// ~keep There are two distinct whys and they demand opposite operator actions:
/// [`UNPUBLISHED_VERSION_REASON`] (registry mode resolving a version this run itself
/// produces — wait for the release) and [`MISSING_TOOLCHAIN_REASON`] (the formatter's
/// executable is absent — install it, and treat this run's output as unformatted). A
/// heading that hard-coded "until the pinned version is published" was correct for the
/// first and actively false for the second, and it is the heading an operator triages on:
/// filed under benign release-cycle noise, a skipped formatter's own entry goes unread and
/// the unformatted tree gets committed. Missing toolchains therefore get their own heading
/// that says the output was left unformatted.
fn report_deferred(crate_name: Option<&str>, deferred: &[DeferredFormatting]) {
    let scope = crate_name.map_or_else(String::new, |name| format!("[{name}] "));
    let (missing, unpublished): (Vec<_>, Vec<_>) = deferred.iter().partition(|entry| entry.is_missing_toolchain());
    if !missing.is_empty() {
        warn!(
            "{scope}{} formatting step(s) skipped — the executable is not installed, so that \
             output is NOT formatted. Install the toolchain, or re-run with --strict to make \
             this fatal:",
            missing.len()
        );
        for entry in missing {
            warn!("  {entry}");
        }
    }
    if !unpublished.is_empty() {
        warn!(
            "{scope}{} dependency-resolution step(s) deferred until the pinned version is published:",
            unpublished.len()
        );
        for entry in unpublished {
            warn!("  {entry}");
        }
    }
}

/// Run a built-in residual step as a direct child process rooted at `dir`.
///
/// Deliberately not routed through [`run_shell`]: `sh -c "(cd {dir} && ...)"` cannot
/// carry a Windows path. `canonicalize` yields the extended-length form `\\?\C:\...`,
/// which a POSIX `cd` rejects, so the shell exits 1 from the failed `cd` before the
/// tool is ever reached -- and 1 is not [`SHELL_COMMAND_NOT_FOUND`], so an absent
/// toolchain arrived as "the formatter ran and rejected the code" and killed the run.
/// Letting the OS set the working directory also drops the unquoted `{dir}`
/// interpolation, so a path containing a space no longer truncates the command. ~keep
fn run_in_dir(program: &str, args: &[&str], dir: &Path, lang: &str) -> Result<(), ShellFailure> {
    let step = std::iter::once(program)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ");
    match std::process::Command::new(program).args(args).current_dir(dir).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(ShellFailure {
            executable_missing: false,
            error: anyhow::anyhow!(
                "formatter for {lang} exited with {status}: {step} (in {})",
                dir.display()
            ),
        }),
        Err(error) => Err(ShellFailure {
            executable_missing: error.kind() == std::io::ErrorKind::NotFound,
            error: anyhow::Error::new(error).context(format!("failed to run {step} for {lang} in {}", dir.display())),
        }),
    }
}

/// Populate `go.sum` from `go.mod` in the e2e Go directory.
fn run_go_mod_tidy(dir: &Path, lang: &str, strict: bool, deferred: &mut Vec<DeferredFormatting>) -> anyhow::Result<()> {
    match run_in_dir("go", &["mod", "tidy"], dir, "go") {
        Ok(()) => Ok(()),
        Err(failure) => resolve_shell_failure(failure, lang, GO_MOD_TIDY_STEP, strict, deferred),
    }
}

/// Format `.ex`/`.exs` in the e2e Elixir directory with `mix format`.
///
/// Must run from `dir` so mix reads that project's own `.formatter.exs` (emitted
/// alongside `mix.exs`) — a bare `mix format` has no `inputs:` without it. That
/// file deliberately omits `import_deps`, so this needs no prior `mix deps.get`.
fn run_mix_format(dir: &Path, lang: &str, strict: bool, deferred: &mut Vec<DeferredFormatting>) -> anyhow::Result<()> {
    match run_in_dir("mix", &["format"], dir, "elixir") {
        Ok(()) => Ok(()),
        Err(failure) => resolve_shell_failure(failure, lang, "mix format", strict, deferred),
    }
}

#[cfg(test)]
#[path = "format_tests.rs"]
mod tests;
