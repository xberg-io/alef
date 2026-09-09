mod stamp_gate;

pub(crate) use stamp_gate::generated_tree_needs_formatting;
pub use stamp_gate::unstamp_before_formatting;

use crate::core::config::{Language, OutputLayout, ResolvedCrateConfig};
use crate::e2e::format::DeferredFormatting;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, warn};

/// Reason recorded when a formatting step could not run because its executable is not
/// installed on this machine.
///
/// Written out verbatim rather than shared, because `e2e::format`'s constant of the same
/// name is private to that module and [`DeferredFormatting::is_missing_toolchain`]
/// classifies by exact string equality. An approximate copy would file every skipped
/// package formatter under the reporter's "waiting for a publish" heading -- the precise
/// false-heading bug that reporter's own split exists to prevent -- so
/// `a_recorded_skip_is_classified_as_a_missing_toolchain` fails the moment the two
/// spellings drift apart. ~keep
const MISSING_TOOLCHAIN_REASON: &str = "the formatter's executable is not installed on this machine; generation \
                                        continued so the run still reaches finalisation. Install the toolchain, or \
                                        re-run with --strict to make this fatal";

/// Scope recorded for a formatter that shapes the generated package tree as a whole
/// (`poly fmt`, `cargo fmt --all`, the workspace-wide `cargo sort`) rather than one
/// language's output. Per-language residuals record their own language instead.
const PACKAGE_TREE_SCOPE: &str = "packages";

/// One pass of [`format_generated`]: how it resolves executable presence, and every step
/// it had to skip because that executable was absent.
///
/// Skips used to be a fire-and-forget `warn!` at each site, which made them invisible to
/// the caller and therefore impossible for `--strict` to escalate -- the shipped bindings
/// under `packages/<lang>` were the one formatting surface `--strict` did not actually
/// guard. Recording them as the same [`DeferredFormatting`] the e2e stage already emits
/// gives both surfaces one record type and one policy.
///
/// The probe is injectable so that policy is tested against a controlled toolchain rather
/// than whatever the host running the suite happens to have installed. ~keep
struct FormatPass<'probe> {
    is_available: &'probe dyn Fn(&str) -> bool,
    skipped: Vec<DeferredFormatting>,
}

impl<'probe> FormatPass<'probe> {
    fn new(is_available: &'probe dyn Fn(&str) -> bool) -> Self {
        Self {
            is_available,
            skipped: Vec::new(),
        }
    }

    fn available(&self, tool: &str) -> bool {
        (self.is_available)(tool)
    }

    /// Record that `step` did not run because `tool` is not installed. The tool name goes
    /// into the step text because the reason field is a fixed literal -- it is what makes
    /// the record classifiable -- and an operator still has to be told which binary to
    /// install when a step names more than one (`cargo fmt --all` needs both `cargo` and
    /// `rustfmt`). ~keep
    fn record_missing(&mut self, scope: &str, tool: &str, step: &str) {
        self.skipped.push(DeferredFormatting {
            language: scope.to_owned(),
            step: format!("{step} (missing: {tool})"),
            reason: MISSING_TOOLCHAIN_REASON.to_owned(),
        });
    }
}

/// One residual formatter invocation poly cannot perform (project-wide tools that
/// don't fit poly's per-file model): `cargo sort`, `mix format`, `dotnet format`.
#[derive(Debug)]
struct ResidualStep {
    command: String,
    args: Vec<String>,
    work_dir: PathBuf,
}

/// A code formatter that shapes generated output and must be present for a run.
#[derive(Clone, Copy)]
struct RequiredFormatter {
    tool: &'static str,
    install_hint: &'static str,
}

/// The formatters whose presence is required for deterministic generation of
/// `languages`.
///
/// `rustfmt` and `poly` are always required: every binding emits a Rust glue
/// crate that rustfmt reflows, and poly formats each language's emitted package.
/// `cargo-sort` is required only when a language whose residual pass runs
/// `cargo sort` is generated (wasm, ffi, ruby, elixir, r). `mix` is required only
/// when Elixir is generated: poly's own pass excludes `.ex`/`.exs` files (see
/// [`POLY_ELIXIR_EXCLUDE_GLOBS`]) because its pure-Rust Elixir formatter misindents
/// them, so `mix format` is the sole formatter for that output and its absence
/// must not pass unnoticed.
fn required_formatters(languages: &[Language]) -> Vec<RequiredFormatter> {
    let mut required = vec![
        RequiredFormatter {
            tool: "rustfmt",
            install_hint: "rustup component add rustfmt",
        },
        RequiredFormatter {
            tool: "poly",
            install_hint: "install polylint (`poly`) and put it on PATH",
        },
    ];
    let needs_cargo_sort = languages.iter().any(|language| {
        matches!(
            language,
            Language::Wasm | Language::Ffi | Language::Ruby | Language::Elixir | Language::R
        )
    });
    if needs_cargo_sort {
        required.push(RequiredFormatter {
            tool: "cargo-sort",
            install_hint: "cargo install cargo-sort",
        });
    }
    if languages.contains(&Language::Elixir) {
        required.push(RequiredFormatter {
            tool: "mix",
            install_hint: "install Elixir (https://elixir-lang.org/install.html); `mix` ships with it",
        });
    }
    required
}

/// Warn (never fail) when a formatter that shapes generated output is missing
/// from PATH.
///
/// alef always applies formatting when the tools are present — poly in
/// particular formats through `poly fmt` whenever it is on PATH and the pass is
/// skipped otherwise. A missing formatter (`rustfmt`, `poly`, `cargo-sort`, or
/// `mix`) can leave output un(der)-formatted and host-dependent, which may trip
/// the freshness check (#184); rather than abort generation, warn and name each
/// missing tool and how to install it so the operator can restore deterministic
/// output. `mix` in particular has no fallback: unlike the other residuals it is
/// the *sole* formatter for `.ex`/`.exs` output (poly is deliberately excluded,
/// see [`POLY_ELIXIR_EXCLUDE_GLOBS`]), so a missing `mix` means generated Elixir
/// source is left completely unformatted, not merely under-formatted.
pub fn warn_missing_formatters(languages: &[Language]) {
    let missing: Vec<RequiredFormatter> = required_formatters(languages)
        .into_iter()
        .filter(|formatter| !is_tool_available(formatter.tool))
        .collect();
    if missing.is_empty() {
        return;
    }
    let details = missing
        .iter()
        .map(|formatter| format!("  - {}: {}", formatter.tool, formatter.install_hint))
        .collect::<Vec<_>>()
        .join("\n");
    warn!(
        "code formatter(s) not found on PATH; generated output may be un(der)-formatted and \
         host-dependent (#184). Install to restore deterministic formatting:\n{details}"
    );
}

/// Run language-native formatters on emitted packages after generation.
///
/// Formatting is always delegated to the `poly` (polylint) CLI. On a full regen
/// (`only_languages = None`, the `alef all` path) this converges to a fixed point:
/// see [`converge_full_regen_formatting`]. On a partial regen (a single language's
/// files changed) a single `poly fmt --fix` pass runs over the changed language's
/// package directory, followed by that language's residual native pass for the
/// project-wide tools poly cannot wrap (wasm/ruby/elixir/R native crate sort, plus
/// `mix format` for Elixir's `.ex`/`.exs` source — poly is excluded from those,
/// see [`POLY_ELIXIR_EXCLUDE_GLOBS`]).
///
/// Best-effort: a missing `poly` binary, a poly error, or a missing residual tool
/// is logged as a warning and never aborts the generate command.
///
/// Callers that expose a `--strict` flag must use [`format_generated_reporting`] instead:
/// this entry point discards the skip records, which is exactly how the shipped bindings
/// ended up being the one formatting surface `--strict` did not guard.
pub fn format_generated(config: &ResolvedCrateConfig, base_dir: &Path, only_languages: Option<&HashSet<Language>>) {
    let skipped = run_format_pass(config, base_dir, only_languages, &is_tool_available);
    crate::e2e::format::warn_deferred(&skipped);
}

/// [`format_generated`], returning every step that could not run because its executable is
/// absent, and failing the run when `strict` asks for it.
///
/// This is the strict-aware entry point every command with a `--strict` flag calls, so
/// `alef generate --strict` and `alef all --strict` give the same answer to the same
/// question. `--strict` is deliberately not the default: `poly`, `rustfmt`, `cargo-sort`
/// and `mix` are host toolchains a contributor may legitimately lack, and making a missing
/// one fatal by default breaks every fresh clone. The sanctioned shape is warn + record,
/// with `--strict` escalating -- identical to the e2e formatter's own contract, down to
/// the record type. ~keep
pub fn format_generated_reporting(
    config: &ResolvedCrateConfig,
    base_dir: &Path,
    only_languages: Option<&HashSet<Language>>,
    strict: bool,
) -> anyhow::Result<Vec<DeferredFormatting>> {
    format_generated_reporting_with(config, base_dir, only_languages, strict, &is_tool_available)
}

/// Testable seam for [`format_generated_reporting`]: resolves executable presence through
/// `is_available` instead of PATH, so the `--strict` escalation is provable without
/// depending on which formatters the host running the suite happens to have. ~keep
pub(crate) fn format_generated_reporting_with(
    config: &ResolvedCrateConfig,
    base_dir: &Path,
    only_languages: Option<&HashSet<Language>>,
    strict: bool,
    is_available: &dyn Fn(&str) -> bool,
) -> anyhow::Result<Vec<DeferredFormatting>> {
    let skipped = run_format_pass(config, base_dir, only_languages, is_available);
    crate::e2e::format::warn_deferred(&skipped);
    escalate_missing_toolchains(skipped, strict)
}

/// The single `--strict` policy shared by both formatting surfaces: a formatter that is
/// merely absent is survived by default and fatal under `strict`. A formatter that RAN and
/// rejected the code is not represented here at all -- that already fails regardless.
fn escalate_missing_toolchains(
    skipped: Vec<DeferredFormatting>,
    strict: bool,
) -> anyhow::Result<Vec<DeferredFormatting>> {
    let missing: Vec<String> = skipped
        .iter()
        .filter(|entry| entry.is_missing_toolchain())
        .map(|entry| format!("[{}] {}", entry.language, entry.step))
        .collect();
    if !strict || missing.is_empty() {
        return Ok(skipped);
    }
    anyhow::bail!(
        "--strict: {} formatting step(s) could not run because their executable is not installed, so the \
         generated packages are NOT formatted: {}",
        missing.len(),
        missing.join("; ")
    )
}

fn run_format_pass(
    config: &ResolvedCrateConfig,
    base_dir: &Path,
    only_languages: Option<&HashSet<Language>>,
    is_available: &dyn Fn(&str) -> bool,
) -> Vec<DeferredFormatting> {
    let mut pass = FormatPass::new(is_available);
    // `None` (full regen, the `alef all` path) always runs the whole-tree convergence pass
    // below, formatting every generated package under `base_dir` regardless of `only_languages`.
    // The `Some(_)` (partial regen) branch below is what tells `poly_paths` which package
    // directories to format -- see its own comment for why `poly_langs` is derived from `only`
    // directly rather than from a `files` list. ~keep
    match only_languages {
        None => converge_full_regen(base_dir, &mut pass),
        // `poly_langs` comes directly from the caller's `only` set, not from which languages
        // happen to be keys in `files`. It used to be `files.iter().filter(|lang|
        // only.contains(lang))` -- an intersection that silently dropped a language `only`
        // named but `files` had no entry for at all. A post-build step that writes straight to
        // disk (Swift's `MaterializeSwiftBridge` for `RustBridgeC.h`) is exactly that case on a
        // run where nothing else regenerated: the caller adds the language to `only` precisely
        // because that post-build output still needs formatting, but `files` (bindings + stubs)
        // has no entry for it at all, so the old intersection produced an empty `poly_langs` and
        // returned before ever calling `poly_paths` -- the caller's `only` decision was
        // silently discarded and the post-build output shipped never formatted. `only` is
        // already the caller's complete, authoritative answer to "what needs formatting this
        // run"; re-deriving it from `files` a second time can only narrow it incorrectly, never
        // usefully. ~keep
        Some(only) => {
            let poly_langs: Vec<Language> = only.iter().copied().collect();
            if poly_langs.is_empty() {
                return pass.skipped;
            }
            let paths = poly_paths(config, base_dir, only_languages, &poly_langs);
            poly_format_pass(&paths, base_dir, &mut pass);
            for &lang in &poly_langs {
                let lang_str = lang.to_string().to_lowercase();
                for step in language_residuals(config, lang, base_dir) {
                    run_residual(&step, &lang_str, &mut pass);
                }
            }
        }
    }
    pass.skipped
}

/// Maximum `poly fmt --fix` passes attempted while converging a full regen.
///
/// Some poly-bundled engines (`.cs`, `.java`, `.json` today) are not single-pass
/// idempotent on freshly generated output: a first `poly fmt --fix` pass can still
/// leave `poly fmt --check` reporting drift. Looping converges them so a full
/// regen is committable without a manual cleanup pass downstream (see #184-style
/// freshness-check failures).
const MAX_POLY_FMT_PASSES: u32 = 3;

/// Self-cleaning full-regen formatting pass, used on the `alef all` path
/// (`only_languages = None`).
///
/// Loops `poly fmt --fix <base_dir>` to a fixed point (detected via `poly fmt
/// --check`, bounded by [`MAX_POLY_FMT_PASSES`]), folding a workspace-wide
/// `cargo fmt --all` and a workspace-wide `cargo sort -n -w` into *every* pass of
/// the same loop. Running them inside the loop — rather than once, after —
/// means that if either tool disagrees with poly's own formatting, the next
/// pass's `poly fmt --fix`/`--check` observes and reconciles the drift instead of
/// leaving the tree dirty.
///
/// This replaces the old per-language cargo-sort residuals on a full regen: those
/// only covered the language whose crate directory they targeted (and the
/// workspace-wide `-w` variant ran only when the ffi target was generated),
/// leaving other generated crates (python, node, php, swift, dart, …) unsorted —
/// exactly the gap that trips poly's own workspace-wide cargo-sort check
/// downstream. A single `cargo sort -n -w` at the repo root covers every crate in
/// the workspace regardless of which languages this run generated.
///
/// `.ex`/`.exs` output is handled outside this loop entirely: poly's own pass
/// excludes them (see [`POLY_ELIXIR_EXCLUDE_GLOBS`]), so [`run_elixir_mix_format`]
/// runs once, after the loop settles, regardless of whether poly converged --
/// the two concerns are independent and neither blocks the other.
///
/// Best-effort throughout: a missing `poly`, `cargo`, `rustfmt`, `cargo-sort`, or
/// `mix` is a warning, never a failure, and generation is never aborted.
pub(crate) fn converge_full_regen_formatting(base_dir: &Path) {
    let mut pass = FormatPass::new(&is_tool_available);
    converge_full_regen(base_dir, &mut pass);
    crate::e2e::format::warn_deferred(&pass.skipped);
}

/// [`converge_full_regen_formatting`]'s body, recording absent executables into `pass`
/// instead of dropping them into a `warn!` no caller can see.
fn converge_full_regen(base_dir: &Path, pass: &mut FormatPass<'_>) {
    let poly_present = pass.available("poly");
    if !poly_present {
        pass.record_missing(PACKAGE_TREE_SCOPE, "poly", POLY_FMT_STEP);
    }
    let root = vec![base_dir.to_path_buf()];

    for _iteration in 1..=MAX_POLY_FMT_PASSES {
        if poly_present {
            poly_format_pass(&root, base_dir, pass);
        }
        run_cargo_fmt(base_dir, pass);
        run_workspace_cargo_sort(base_dir, pass);

        if !poly_present || poly_fmt_is_clean(base_dir) {
            run_elixir_mix_format(base_dir, pass);
            return;
        }
    }
    warn!(
        "poly fmt did not converge after {MAX_POLY_FMT_PASSES} passes (non-fatal); generated \
         output may have residual formatting drift"
    );
    run_elixir_mix_format(base_dir, pass);
}

/// Check `poly fmt --check <base_dir>` for a clean (already-formatted) tree. Used
/// only to detect convergence inside [`converge_full_regen_formatting`]'s loop.
///
/// Excludes the same Elixir globs as [`poly_format`] (see
/// [`POLY_ELIXIR_EXCLUDE_GLOBS`]): without this, the check would judge
/// `mix format`'s correct output by poly's own (incompatible) Elixir formatting
/// opinion and never report clean, spinning the convergence loop to its cap on
/// every full regen that generates Elixir.
///
/// Deliberately without `--fix-generated`, unlike [`stamp_gate::generated_tree_needs_formatting`]:
/// this runs *inside* the format pass, after `unstamp_before_formatting` has already cleared the
/// stamp from everything this run is allowed to reformat. Adding the flag here would make the loop
/// spin on stamped files the paired `poly fmt --fix` is not touching, and never report clean. ~keep
fn poly_fmt_is_clean(base_dir: &Path) -> bool {
    let path_str = base_dir.to_string_lossy().into_owned();
    let mut args: Vec<String> = vec!["fmt".to_owned(), "--check".to_owned(), path_str];
    push_poly_format_excludes(&mut args);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_formatter("poly", &arg_refs, base_dir).is_ok()
}

/// Run `cargo fmt --all` at the workspace root, when `cargo`, `rustfmt`, and a
/// root `Cargo.toml` are all present. Folded into
/// [`converge_full_regen_formatting`]'s loop rather than run once afterward, so a
/// later `poly fmt` pass reconciles anything cargo fmt changes that poly's own
/// per-file rustfmt invocation did not already produce. Best-effort: a missing
/// root `Cargo.toml` is a debug/skip (not every generated tree is a cargo
/// workspace); a missing tool is a warning/skip; a non-zero exit is a warning.
fn run_cargo_fmt(base_dir: &Path, pass: &mut FormatPass<'_>) {
    if !base_dir.join("Cargo.toml").exists() {
        debug!(
            "no root Cargo.toml at {}, skipping workspace cargo fmt",
            base_dir.display()
        );
        return;
    }
    for tool in ["cargo", "rustfmt"] {
        if !pass.available(tool) {
            pass.record_missing(PACKAGE_TREE_SCOPE, tool, CARGO_FMT_STEP);
            return;
        }
    }
    match run_formatter("cargo", &["fmt", "--all"], base_dir) {
        Ok(()) => debug!("cargo fmt --all ok"),
        Err(e) => warn!("cargo fmt --all failed (non-fatal): {e}"),
    }
}

/// Run `cargo sort -n -w` once at the workspace root, covering every crate in the
/// workspace regardless of which languages this run generated. See
/// [`converge_full_regen_formatting`] for why this replaces the per-language
/// residuals on a full regen. The `-n` flag skips cargo-sort's own post-sort
/// formatting pass (which would otherwise fight poly's TOML formatter over
/// whitespace/quote style); it does not affect table or dependency ordering, so
/// it does not change what poly's bundled cargo-sort check accepts as sorted.
/// Best-effort: a missing root `Cargo.toml` is a debug/skip; a missing
/// `cargo-sort` binary is a warning/skip; a non-zero exit is a warning.
fn run_workspace_cargo_sort(base_dir: &Path, pass: &mut FormatPass<'_>) {
    if !base_dir.join("Cargo.toml").exists() {
        debug!(
            "no root Cargo.toml at {}, skipping workspace cargo sort",
            base_dir.display()
        );
        return;
    }
    if !pass.available("cargo-sort") {
        pass.record_missing(PACKAGE_TREE_SCOPE, "cargo-sort", CARGO_SORT_STEP);
        return;
    }
    match run_formatter("cargo", &["sort", "-n", "-w"], base_dir) {
        Ok(()) => debug!("cargo sort -n -w ok"),
        Err(e) => warn!("cargo sort -n -w failed (non-fatal): {e}"),
    }
}

/// Run `poly lint <base_dir>`. Propagates failure — a non-zero exit is an error.
///
/// Unlike the best-effort formatting steps in this module (each one tool among several,
/// individually skippable and escalated only under `--strict`), `poly` is the entire
/// implementation of `alef lint` -- there is no partial coverage to fall back to when it is
/// missing. Warning and returning `Ok(())` here used to report a clean lint pass for a run
/// that checked nothing, with no `--strict` equivalent available to catch it (unlike `alef
/// generate`/`alef all`, which do escalate a missing `poly` under `--strict`). ~keep
pub fn poly_lint(base_dir: &Path) -> anyhow::Result<()> {
    poly_lint_with(base_dir, &is_tool_available)
}

/// Testable seam for [`poly_lint`]: resolves `poly`'s presence through `is_available` instead
/// of PATH, the same seam [`format_generated_reporting_with`] uses, so the missing-`poly` bail
/// is provable without depending on whether the host running the suite happens to have `poly`
/// installed. ~keep
pub(crate) fn poly_lint_with(base_dir: &Path, is_available: &dyn Fn(&str) -> bool) -> anyhow::Result<()> {
    if !is_available("poly") {
        anyhow::bail!("poly not found on PATH; \"alef lint\" has nothing else to run -- install poly to lint");
    }
    let path_str = base_dir.to_string_lossy().into_owned();
    let arg_refs: Vec<&str> = vec!["lint", &path_str];
    match run_formatter("poly", &arg_refs, base_dir) {
        Ok(()) => {
            debug!("poly lint ok");
            Ok(())
        }
        Err(e) => Err(anyhow::anyhow!("poly lint failed: {e}")),
    }
}

/// Paths to hand to poly. Full regen → the repo root (one pass). Partial regen →
/// every directory each changed language generates into (existing dirs only, deduped,
/// with nested entries collapsed into their enclosing directory).
///
/// `package_dir(lang)`, `output_for(lang)`, *and* the binding crate root implied by
/// `output_for(lang)`, deliberately: this is the same span `generate_sweep_roots` reclaims
/// orphans across and the same span `finalize_hashes` stamps, and a formatting scope narrower
/// than the stamping scope means alef stamps bytes it never canonicalised. For most languages
/// all three coincide or nest -- but `package_dir(Python)` is the wheel at `packages/python`
/// while the PyO3 glue crate is generated into `crates/<name>-py/src`, so `alef generate --lang
/// python` used to stamp a Rust file no formatter had touched, and the next whole-tree pass
/// immediately made that stamp stale.
///
/// `output_for(lang)` alone is not enough either: it names the *source* directory
/// (`crates/<name>-py/src`), one level below the crate root that actually holds the binding
/// crate's own `Cargo.toml`. Any language whose default output template is
/// `<crate-root>/src` (see [`crate::core::config::resolve_helpers::default_binding_crate_root`]
/// -- today python, ffi, and php, alongside node/wasm whose `package_dir` already resolves to
/// the same crate root) generates a manifest that lived outside every formatting pass on a
/// partial regen: `poly fmt` never saw it, so it shipped non-canonical from generation, and
/// `alef verify` only caught the drift the first time something else reformatted the file.
/// [`crate::core::config::OutputLayout::from_output_dir`] is the same root-vs-src split the FFI
/// and Wasm backends already use to locate their own manifests, so recovering the crate root
/// through it here (rather than hard-coding which languages have a `<root>/src` shape) keeps
/// this generic across every current and future binding-crate language. Format scope must
/// equal stamp scope. ~keep
fn poly_paths(
    config: &ResolvedCrateConfig,
    base_dir: &Path,
    only_languages: Option<&HashSet<Language>>,
    poly_langs: &[Language],
) -> Vec<PathBuf> {
    match only_languages {
        None => vec![base_dir.to_path_buf()],
        Some(_) => {
            let mut seen = HashSet::new();
            let mut dirs = Vec::new();
            for &lang in poly_langs {
                let package_dir = base_dir.join(config.package_dir(lang));
                let output_path = config.output_for(&lang.to_string());
                let output_dir = output_path.map(|out| base_dir.join(out));
                let crate_root = output_path
                    .map(|out| OutputLayout::from_output_dir(&out.to_string_lossy()).root)
                    .map(|root| base_dir.join(root));
                for dir in std::iter::once(package_dir).chain(output_dir).chain(crate_root) {
                    if seen.insert(dir.clone()) && dir.exists() {
                        dirs.push(dir);
                    }
                }
            }
            collapse_nested_paths(dirs)
        }
    }
}

/// Which of `languages` own any path in `changed_paths`, using the exact same
/// package_dir/output_for/crate_root directories [`poly_paths`] hands to poly for a partial
/// regen.
///
/// A scaffold-managed manifest (`packages/java/pom.xml`, `crates/<name>-ffi/cmake/*.cmake`,
/// `packages/python/pyproject.toml`) can change with no corresponding write in the
/// bindings/service-api/public-api/stubs phases -- e.g. a `package_metadata.license` edit
/// rewrites every language's manifest but touches no generated source at all. Those phases
/// are the only places `Commands::Generate` (`bin_cli/core_commands.rs`) inserts into
/// `changed_languages`, so a scaffold-only write used to leave that language out of
/// `format_scope` entirely: `reconcile_managed_scaffold_manifests`'s write reached disk
/// unformatted, and no later pass in a partial `alef generate` run ever saw it. `alef all`'s
/// full-tree convergence pass (`converge_full_regen`, `only_languages = None`) covers every
/// byte under `base_dir` regardless of which phase wrote it, which is why the identical
/// license edit through `alef all` never reproduced this. ~keep
pub(crate) fn languages_owning_changed_paths(
    config: &ResolvedCrateConfig,
    base_dir: &Path,
    languages: &[Language],
    changed_paths: &HashSet<PathBuf>,
) -> HashSet<Language> {
    if changed_paths.is_empty() {
        return HashSet::new();
    }
    let mut owners = HashSet::new();
    for &lang in languages {
        let single = HashSet::from([lang]);
        let dirs = poly_paths(config, base_dir, Some(&single), &[lang]);
        if changed_paths
            .iter()
            .any(|path| dirs.iter().any(|dir| path.starts_with(dir)))
        {
            owners.insert(lang);
        }
    }
    owners
}

/// Drop every path that lies inside another path in the list. poly walks each root it is
/// given recursively, so handing it both `crates/x-node` and `crates/x-node/src` would
/// format the same subtree twice -- and a formatter run twice in one pass is how
/// non-idempotent engines produce output that differs from what a single `--check` expects.
fn collapse_nested_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths
        .iter()
        .filter(|candidate| {
            !paths
                .iter()
                .any(|other| other != *candidate && candidate.starts_with(other))
        })
        .cloned()
        .collect()
}

/// Glob patterns excluded from every poly `fmt` invocation, `--fix` and `--check`
/// alike (see [`poly_format`] and [`poly_fmt_is_clean`]): `.ex`/`.exs` sources are
/// formatted solely by `mix format` (see `language_residuals`'s `Language::Elixir`
/// arm and [`run_elixir_mix_format`]).
///
/// poly's pure-Rust Elixir formatter misindents constructs that `mix format`
/// emits correctly — multi-line struct/map field continuation collapses from
/// mix's canonical 10-space width to flush-left 2-space, and `|>` pipe
/// continuation drops from 6 spaces to 4 — and then reports its own corrupted
/// output as `--check`-clean, so no freshness gate ever catches the drift.
/// Excluding poly from `.ex`/`.exs` entirely, rather than reformatting after it,
/// avoids that same class of bug recurring: a `--check` pass that still
/// considers itself authoritative over files it no longer formats.
///
/// Anchored with a bare `**/` prefix (no `packages/elixir/` path component) so
/// the same glob excludes correctly regardless of which root poly is given: the
/// repo root on a full regen, or the `packages/elixir` package directory itself
/// on a partial regen.
const POLY_ELIXIR_EXCLUDE_GLOBS: [&str; 2] = ["**/*.ex", "**/*.exs"];

/// C# source, excluded from poly for a different reason than Elixir: not corruption, but
/// *irreproducibility*.
///
/// Poly delegates `.cs` to an external `clang-format`, whose output is not stable across its own
/// versions. Two machines on identical alef and identical poly commit different bytes for the same
/// generated file — clang-format 18.1.8 and 23.1.0 each reproduce a different committed blob,
/// differing in nullable-switch layout — so the freshness gate fails for a reason no consumer can
/// see in its own diff, and the remedy looks like "pin clang-format in every consumer's CI".
///
/// alef already emits C# in the layout `dotnet format whitespace` accepts, and
/// `generated_csharp_uses_formatter_stable_layout` asserts exactly that against the real formatter.
/// So the emission is already canonical, and letting a second formatter reflow it afterwards can
/// only move it off that contract — differently per version. Excluding poly leaves the committed
/// bytes equal to what alef emits, which is the same on every machine.
///
/// Excluding rather than reformatting-after, for the reason the Elixir globs above give: a
/// `--check` pass that still considers itself authoritative over files it no longer formats is the
/// bug class, not the fix. Anchored with a bare `**/` for the same root-independence reason. ~keep
const POLY_CSHARP_EXCLUDE_GLOBS: [&str; 1] = ["**/*.cs"];

/// Append `--exclude <glob>` for every glob poly must not format: [`POLY_ELIXIR_EXCLUDE_GLOBS`]
/// and [`POLY_CSHARP_EXCLUDE_GLOBS`].
fn push_poly_format_excludes(args: &mut Vec<String>) {
    for glob in POLY_ELIXIR_EXCLUDE_GLOBS.iter().chain(POLY_CSHARP_EXCLUDE_GLOBS.iter()) {
        args.push("--exclude".to_owned());
        args.push((*glob).to_owned());
    }
}

/// Format `paths` by invoking the `poly` CLI (`poly fmt --fix`), rewriting changed
/// files in place. `config_start` is poly's working directory; it walks up from
/// there for `poly.toml`. Best-effort: a missing `poly` binary or a non-zero exit
/// is logged and never propagated (matching the per-language formatter contract).
///
/// Excludes `.ex`/`.exs` (see [`POLY_ELIXIR_EXCLUDE_GLOBS`]) so `mix format`
/// remains their sole formatter.
///
/// Executable permission bits are snapshotted before the pass and restored after:
/// poly rewrites changed files via atomic rename, which resets the mode to `0644`
/// and silently strips the exec bit from every generated shebang script it
/// reformats (`run_tests.php`, `download_ffi.sh`, `mvnw`, `gradlew`, …) — which
/// poly's own `file-safety` lint then rejects on the next commit.
pub(crate) fn poly_format(paths: &[PathBuf], config_start: &Path) {
    let mut pass = FormatPass::new(&is_tool_available);
    poly_format_pass(paths, config_start, &mut pass);
    crate::e2e::format::warn_deferred(&pass.skipped);
}

/// [`poly_format`] recording an absent `poly` into `pass`.
///
/// Checked here rather than left to [`poly_format_strict`]'s own bail because that bail
/// cannot tell an absent executable from poly running and rejecting the code, and only the
/// first of those is something `--strict` should let an operator escalate separately. ~keep
fn poly_format_pass(paths: &[PathBuf], config_start: &Path, pass: &mut FormatPass<'_>) {
    if paths.is_empty() {
        return;
    }
    if !pass.available("poly") {
        pass.record_missing(PACKAGE_TREE_SCOPE, "poly", POLY_FMT_STEP);
        return;
    }
    if let Err(error) = poly_format_strict(paths, config_start) {
        warn!("poly fmt failed (non-fatal): {error}");
    }
}

/// Step names recorded when a whole-tree formatter is skipped for want of its executable.
const POLY_FMT_STEP: &str = "poly fmt --fix";
const CARGO_FMT_STEP: &str = "cargo fmt --all";
const CARGO_SORT_STEP: &str = "cargo sort -n -w";

pub(crate) fn poly_format_strict(paths: &[PathBuf], config_start: &Path) -> anyhow::Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    if !is_tool_available("poly") {
        anyhow::bail!("poly not found on PATH; generated output cannot be formatted");
    }
    let executable_modes = snapshot_executable_modes(paths);
    let mut args: Vec<String> = vec!["fmt".to_owned(), "--fix".to_owned()];
    args.extend(paths.iter().map(|path| path.to_string_lossy().into_owned()));
    push_poly_format_excludes(&mut args);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = run_poly_formatter(&arg_refs, config_start);
    restore_executable_modes(&executable_modes);
    result?;
    debug!("poly fmt over {} path(s) ok", paths.len());
    Ok(())
}

fn run_poly_formatter(args: &[&str], work_dir: &Path) -> anyhow::Result<()> {
    let output = Command::new("poly").args(args).current_dir(work_dir).output()?;
    if poly_format_exit_code_is_success(output.status.code()) && !poly_format_output_reports_failure(&output.stderr) {
        return Ok(());
    }
    Err(formatter_failure(&output))
}

fn poly_format_exit_code_is_success(exit_code: Option<i32>) -> bool {
    matches!(exit_code, Some(0 | 1))
}

fn poly_format_output_reports_failure(stderr: &[u8]) -> bool {
    String::from_utf8_lossy(stderr).contains("format failed:")
}

/// Directory names the executable-mode snapshot never descends into. They hold
/// dependency caches and build output — never alef-generated scripts — and
/// walking them on a repo-root pass costs far more than the whole format run.
#[cfg(unix)]
const EXEC_SNAPSHOT_SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".venv",
    "venv",
    "vendor",
    "deps",
    "_build",
    "build",
    ".build",
    "zig-out",
    "dist",
    "__pycache__",
    ".gradle",
    ".dart_tool",
    ".zig-cache",
    ".cache",
];

/// Mode bits granting execute permission to owner, group, or other.
#[cfg(unix)]
const EXECUTE_BITS: u32 = 0o111;

/// Record the mode of every regular file under `paths` that is currently
/// executable, so [`restore_executable_modes`] can put back exactly what was
/// there if `poly fmt` drops it.
#[cfg(unix)]
fn snapshot_executable_modes(paths: &[PathBuf]) -> Vec<(PathBuf, u32)> {
    use std::os::unix::fs::PermissionsExt as _;
    let mut snapshot = Vec::new();
    for root in paths {
        let walker = walkdir::WalkDir::new(root).into_iter().filter_entry(|entry| {
            !entry.file_type().is_dir()
                || entry
                    .file_name()
                    .to_str()
                    .is_none_or(|name| !EXEC_SNAPSHOT_SKIP_DIRS.contains(&name))
        });
        for entry in walker.filter_map(Result::ok) {
            if !entry.file_type().is_file() {
                continue;
            }
            let Ok(metadata) = entry.metadata() else { continue };
            let mode = metadata.permissions().mode();
            if mode & EXECUTE_BITS != 0 {
                snapshot.push((entry.into_path(), mode));
            }
        }
    }
    snapshot
}

/// Re-apply each recorded mode whose execute bits the formatter dropped.
#[cfg(unix)]
fn restore_executable_modes(snapshot: &[(PathBuf, u32)]) {
    use std::os::unix::fs::PermissionsExt as _;
    for (path, mode) in snapshot {
        let Ok(metadata) = std::fs::metadata(path) else {
            continue;
        };
        if metadata.permissions().mode() & EXECUTE_BITS == mode & EXECUTE_BITS {
            continue;
        }
        match std::fs::set_permissions(path, std::fs::Permissions::from_mode(*mode)) {
            Ok(()) => debug!("restored exec bit on {}", path.display()),
            Err(e) => warn!("failed to restore exec bit on {}: {e}", path.display()),
        }
    }
}

#[cfg(not(unix))]
fn snapshot_executable_modes(_paths: &[PathBuf]) -> Vec<(PathBuf, u32)> {
    Vec::new()
}

#[cfg(not(unix))]
fn restore_executable_modes(_snapshot: &[(PathBuf, u32)]) {}

/// Best-effort wiring of poly's git-hook shims (`poly hooks install`) into the
/// generated repo. This installs the pre-commit + commit-msg stages declared in
/// the scaffolded `poly.toml` `[hooks]` section — polylint, polyfmt, file_safety,
/// the `cargo` builtin (clippy / cargo-sort / machete / deny), and the
/// conventional-commit `commit` hook — so every generated repository lints,
/// formats, and validates on commit without any per-repo manual setup.
///
/// No-op when `poly` is absent from PATH or `base_dir` is not a git repository.
/// Idempotent — `poly hooks install` re-writes the same shims, so it is safe to
/// run on every scaffold pass. Never aborts generation.
pub(crate) fn install_poly_hooks(base_dir: &Path) {
    if !base_dir.join(".git").exists() {
        debug!(
            "not a git repository at {}, skipping poly hooks install",
            base_dir.display()
        );
        return;
    }
    if !is_tool_available("poly") {
        warn!("poly not found on PATH (skipping poly hooks install)");
        return;
    }
    match run_formatter("poly", &["hooks", "install"], base_dir) {
        Ok(()) => debug!("poly hooks install ok"),
        Err(e) => warn!("poly hooks install failed (non-fatal): {e}"),
    }
}

/// Build the residual formatter steps for a language — the project-wide tools
/// poly cannot wrap because it works per-file, not per-crate/per-project.
///
/// Most languages' only residual is `cargo sort -n` for binding crates whose
/// `Cargo.toml` is excluded from the root workspace (and therefore from poly's
/// pass) — a dependency-ordering tool (not a formatter) that ships with cargo and
/// is always present in alef's build environment. C# has no residual: it is
/// formatted entirely by poly's deterministic pure-Rust tier-2 tier.
///
/// Elixir is the one exception with two residual concerns: the `cargo sort` for
/// its out-of-workspace native NIF crate, *and* `mix format` for its `.ex`/`.exs`
/// source. Poly's own Elixir engine is excluded from formatting those files at
/// all (see [`POLY_ELIXIR_EXCLUDE_GLOBS`]) because it misindents constructs `mix
/// format` emits correctly and then reports its own corrupted output as
/// `--check`-clean — so unlike every other language here, Elixir has no
/// poly-formatted fallback and a missing `mix` (flagged loudly by
/// [`required_formatters`]) leaves its source completely unformatted, not merely
/// under-formatted. The `mix deps.get` step primes `.formatter.exs`'s
/// `import_deps: [:rustler]` (the scaffolded Elixir `.formatter.exs` imports
/// rustler's formatter rules), which requires the dependency to be fetched into
/// `deps/` before `mix format` can resolve it.
fn language_residuals(config: &ResolvedCrateConfig, lang: Language, base_dir: &Path) -> Vec<ResidualStep> {
    match lang {
        Language::Wasm => {
            let crate_dir = config
                .output_for("wasm")
                .map(resolve_crate_dir)
                .unwrap_or_else(|| Path::new("crates").join(format!("{}-wasm", config.name)));
            let crate_dir_str = crate_dir.to_string_lossy().into_owned().replace('\\', "/");
            vec![cargo_sort(vec![crate_dir_str], base_dir.to_path_buf())]
        }
        Language::Ffi => vec![cargo_sort(vec!["-w".to_owned()], base_dir.to_path_buf())],
        Language::Ruby => {
            // `ruby_native_ext_name`, not `ruby_gem_name`: the extension directory the
            // scaffold actually creates (`ext/{core_crate_dir}_rb/native`) does not track a
            // configured `gem_name` override -- see that method's doc comment for the
            // consumer-reproduced bug this fixes. ~keep
            let ext_name = config.ruby_native_ext_name();
            let native_subdir = format!("ext/{ext_name}/native");
            vec![cargo_sort(vec![native_subdir], base_dir.join("packages/ruby"))]
        }
        Language::Elixir => {
            let app_name = config.elixir_app_name();
            let native_subdir = format!("native/{app_name}_nif");
            let elixir_dir = base_dir.join("packages/elixir");
            vec![
                cargo_sort(vec![native_subdir], elixir_dir.clone()),
                mix_deps_get(elixir_dir.clone()),
                mix_format(elixir_dir),
            ]
        }
        Language::R => vec![cargo_sort(
            vec!["packages/r/src/rust".to_owned()],
            base_dir.to_path_buf(),
        )],
        _ => vec![],
    }
}

/// Construct a `cargo sort -n` residual step. The `-n` flag preserves single-line
/// array formatting, preventing cargo-sort from expanding dependency arrays that
/// alef emits on one line for readability.
fn cargo_sort(mut sort_args: Vec<String>, work_dir: PathBuf) -> ResidualStep {
    let mut args = vec!["sort".to_owned(), "-n".to_owned()];
    args.append(&mut sort_args);
    ResidualStep {
        command: "cargo".to_owned(),
        args,
        work_dir,
    }
}

/// Construct a `mix deps.get` residual step. `.formatter.exs`'s
/// `import_deps: [:rustler]` requires the `rustler` dependency to be fetched into
/// `deps/` before `mix format` can resolve it, so this primes the dependency on
/// every pass rather than assuming a prior `mix deps.get` already ran. Runs
/// through the same best-effort [`run_residual`] as every other step: a network
/// failure here (offline, registry down) is a warning, not an abort, and
/// [`mix_format`] is still attempted afterward — its own failure (e.g. the import
/// dependency genuinely missing) is reported separately, never silently.
fn mix_deps_get(work_dir: PathBuf) -> ResidualStep {
    ResidualStep {
        command: "mix".to_owned(),
        args: vec!["deps.get".to_owned()],
        work_dir,
    }
}

/// Construct a `mix format` residual step — the sole formatter for `.ex`/`.exs`
/// output (poly's own pass excludes them, see [`POLY_ELIXIR_EXCLUDE_GLOBS`]).
fn mix_format(work_dir: PathBuf) -> ResidualStep {
    ResidualStep {
        command: "mix".to_owned(),
        args: vec!["format".to_owned()],
        work_dir,
    }
}

/// Run `mix deps.get` then `mix format` over the generated `packages/elixir`
/// package, if one exists at `base_dir`.
///
/// Used by [`converge_full_regen_formatting`] (the full-regen path), which has no
/// per-language awareness and so cannot go through [`language_residuals`]
/// directly; a partial regen gets the same two steps via that function's
/// `Language::Elixir` arm instead. Existence-gated the same way as
/// [`run_cargo_fmt`]/[`run_workspace_cargo_sort`] gate on a root `Cargo.toml`:
/// a missing `packages/elixir/mix.exs` is a debug/skip (not every crate targets
/// Elixir), not a warning. Both steps are best-effort via [`run_residual`] and
/// never abort generation.
fn run_elixir_mix_format(base_dir: &Path, pass: &mut FormatPass<'_>) {
    let elixir_dir = base_dir.join("packages/elixir");
    if !elixir_dir.join("mix.exs").exists() {
        debug!(
            "no packages/elixir/mix.exs at {}, skipping full-regen mix format",
            base_dir.display()
        );
        return;
    }
    run_residual(&mix_deps_get(elixir_dir.clone()), "elixir", pass);
    run_residual(&mix_format(elixir_dir), "elixir", pass);
}

/// Run a single residual step, best-effort: a missing work dir is a debug/skip, a missing
/// tool is recorded into `pass` (see [`FormatPass`]) and a non-zero exit is a warning.
/// Never aborts generation on its own -- `--strict` escalation happens once, in
/// [`escalate_missing_toolchains`], over everything the pass recorded.
fn run_residual(step: &ResidualStep, lang_str: &str, pass: &mut FormatPass<'_>) {
    if !step.work_dir.exists() {
        debug!(
            "  [{lang_str}] residual work dir does not exist: {}, skipping",
            step.work_dir.display()
        );
        return;
    }
    if !pass.available(&step.command) {
        let command_line = std::iter::once(step.command.as_str())
            .chain(step.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        pass.record_missing(lang_str, &step.command, &command_line);
        return;
    }
    let args: Vec<&str> = step.args.iter().map(String::as_str).collect();
    match run_formatter(&step.command, &args, &step.work_dir) {
        Ok(()) => debug!("  [{lang_str}] {} {:?} ok", step.command, args),
        Err(e) => warn!("[{lang_str}] {} {:?} failed: {e}", step.command, args),
    }
}

/// Check if a tool is available on PATH.
///
/// Resolves via the `which` crate's own PATH walk rather than shelling out to a `which`
/// binary: `which` is not a Windows command, so spawning it there fails outright and
/// [`Result::unwrap_or`] silently reports every tool absent -- a check that "passes" while
/// examining nothing, and formatting gets skipped for the whole run.
pub(crate) fn is_tool_available(tool: &str) -> bool {
    is_tool_available_on(tool, std::env::var_os("PATH"))
}

/// Testable seam for [`is_tool_available`]: resolves `tool` against an explicit `PATH`
/// value instead of the process environment, so a test can prove resolution works from
/// PATH-walking alone -- without a `which`/`where` executable anywhere on that PATH, the
/// exact condition Windows can't satisfy.
fn is_tool_available_on(tool: &str, path_var: Option<std::ffi::OsString>) -> bool {
    which::which_in(tool, path_var, std::env::current_dir().unwrap_or_default()).is_ok()
}

#[path = "format/external_formatter.rs"]
mod external_formatter;
use external_formatter::{formatter_failure, resolve_crate_dir, run_formatter};

#[cfg(test)]
mod scope_tests;
#[cfg(test)]
mod strict_tests;
#[cfg(test)]
mod tests;
