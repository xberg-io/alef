//! Which formatter, if any, alef's own post-write pass applies at a given path -- the single
//! declaration both the writer (`converge_full_regen`, `e2e::format::format_language`) and the
//! reader (`bin_cli::helpers::format_drift::drifted_managed_paths`) consult.
//!
//! # The defect this exists to prevent (alef#478)
//!
//! `alef verify`'s drift check used to hold its own one-line answer to "what formats this
//! path": `poly fmt, always`, applied to a temp sibling of the real file. That answer was wrong
//! in three independent directions at once, and in a consumer repo it manufactured **197**
//! drift findings against a tree two consecutive `alef all` runs left byte-identical:
//!
//! - poly's `[discovery] exclude` is matched against the file's NAME. The real
//!   `crates/<crate>-ffi/Cargo.toml` is excluded by a `**/Cargo.toml` entry; the sibling
//!   `.alef-verify-drift-<rand>.toml` beside it is not, so the check taplo-reformatted a
//!   rendering of a file poly never touches and reported the difference as drift.
//! - The same holds for any tree poly's discovery prunes for a reason the sibling does not
//!   inherit -- a gitignored directory, or a bare `e2e/**` glob that (per poly's own
//!   documentation) also prunes `src/test/java/io/<org>/e2e/`, where `e2e` is a package name.
//! - Residual formatters alef runs *itself* after poly -- `cargo sort -n -w`, `mix format`, a
//!   `[crates.e2e.format]` override that replaces the poly pass outright -- were modelled not
//!   at all. Elixir is the sharpest case: [`poly_exclude_globs`] keeps poly off `.ex`/`.exs` on
//!   BOTH sides, so the sibling was a no-op and the check ended up demanding that disk equal the
//!   raw emitter output, while the writer had run `mix format` over it.
//!
//! The property this module buys, and the reason it is one function rather than a predicate
//! duplicated per call site: adding a residual formatter, a new `[crates.e2e.format]` override,
//! or a new poly exclude **automatically withdraws the reader's claim** over those paths,
//! instead of silently manufacturing findings the writer can never clear. ~keep
//!
//! # Why poly is asked rather than modelled
//!
//! [`PolyCoverage`] answers "does poly's own pass reach this path" by running poly and reading
//! back the paths it reports, rather than by reimplementing gitignore semantics and
//! gitignore-style glob matching in Rust. A reimplementation is a second generator of the same
//! answer and would drift from poly's behaviour the moment poly changes an engine, a default
//! exclude, or its depth-matching rules -- which is the very failure this module exists to end.
//! Probing each writer scope separately preserves poly's root-relative discovery semantics and
//! replaces staging thousands of temp files. E2E scopes cannot be unioned into the repo-root
//! probe: their writer runs with `<output>/<language>` as both target and working directory, so
//! the same repository exclude can classify a path differently in the two contexts. ~keep

use crate::core::config::ResolvedCrateConfig;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Glob patterns excluded from every poly `fmt` invocation alef makes, `--fix` and `--check`
/// alike.
///
/// `.ex`/`.exs`: poly's pure-Rust Elixir formatter misindents constructs `mix format` emits
/// correctly -- multi-line struct/map field continuation collapses from mix's canonical 10-space
/// width to flush-left 2-space, `|>` pipe continuation drops from 6 spaces to 4 -- and then
/// reports its own corrupted output as `--check`-clean, so no freshness gate ever catches the
/// drift. `mix format` is their sole formatter (see `language_residuals`' `Language::Elixir` arm).
///
/// `.cs`: not corruption but *irreproducibility*. poly delegates `.cs` to an external
/// `clang-format` whose output is not stable across its own versions -- clang-format 18.1.8 and
/// 23.1.0 each reproduce a different committed blob -- so the freshness gate fails for a reason
/// no consumer can see in its own diff. alef already emits C# in the layout `dotnet format
/// whitespace` accepts, so letting a second formatter reflow it can only move it off that
/// contract, differently per version.
///
/// Excluding rather than reformatting-after, in both cases, for one reason: a `--check` pass
/// that still considers itself authoritative over files it no longer formats is the bug class,
/// not the fix. Anchored with a bare `**/` prefix (no path component) so the same glob excludes
/// correctly regardless of which root poly is given -- the repo root on a full regen, or a single
/// package directory on a partial one. ~keep
pub(crate) const POLY_EXCLUDE_GLOBS: [&str; 3] = ["**/*.ex", "**/*.exs", "**/*.cs"];

/// The globs above, as the `--exclude <glob>` argument pairs poly takes.
///
/// The one place that spelling is produced. Every poly invocation alef makes goes through it, so
/// the reader's coverage probe and the writer's formatting pass can never disagree about which
/// files poly was allowed to touch. ~keep
pub(crate) fn push_poly_format_excludes(args: &mut Vec<String>) {
    for glob in POLY_EXCLUDE_GLOBS {
        args.push("--exclude".to_owned());
        args.push(glob.to_owned());
    }
}

/// Which formatter, if any, alef's own post-write pass applies at a path.
///
/// [`FormattingOwner::None`] is a positive claim, not an absence of information: nothing alef
/// runs reshapes this path, so the bytes a fresh render produces ARE the bytes that end up on
/// disk, and a reader may compare them directly. The two middle variants are the opposite --
/// they record that alef reshapes the file with a tool whose output it does not model, so no
/// honest byte-level prediction is available and a reader must count the path as unexamined
/// rather than pass it. ~keep
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FormattingOwner {
    /// poly's own `fmt --fix` pass shapes the final bytes here.
    Poly,
    /// A project-wide tool alef runs itself, which poly cannot wrap because it works per-file
    /// rather than per-project. The payload names the command, for the reader's report.
    Residual(&'static str),
    /// A `[crates.e2e.format]` override, which REPLACES the poly pass for its language rather
    /// than running alongside it (`e2e::format::format_language` returns early). The payload is
    /// the configured command, which is opaque shell alef cannot model at all.
    E2eOverride(String),
    /// Nothing alef runs reformats this path.
    None,
}

/// The paths poly's own pass actually reaches in each writer scope, as reported by poly itself.
///
/// `None` means the probe could not run (poly absent, or it failed); callers must treat that as
/// "unknown", never as "covers nothing" -- an empty coverage set and an unavailable probe would
/// otherwise render identically, and the second would silently reclassify every poly-owned path
/// as [`FormattingOwner::None`], which is exactly the vacuous pass this module exists to stop.
pub(crate) struct PolyCoverage {
    scopes: Option<Vec<PolyScope>>,
}

struct PolyScope {
    root: PathBuf,
    covered: Option<HashSet<PathBuf>>,
}

impl PolyCoverage {
    /// Ask poly which files it would process under each root, using that root as both target and
    /// working directory exactly as the corresponding writer pass does. ~keep
    ///
    /// `--fix-generated` is **load-bearing and must never be dropped**. poly skips any file
    /// carrying a `<tool>:hash:<hex>` line (`"hash-stamped generated file (pass --fix-generated
    /// to format)"`), and every path this probe exists to classify is by construction
    /// alef-stamped. Measured on a real consumer tree: without the flag the probe reported
    /// `summary.checked: 6` out of 197 candidates and exited 0 -- a near-total zero-work pass
    /// that renders identically to a clean, fully-examined result. The writer's own pass does not
    /// need the flag because `stamp_gate::unstamp_before_formatting` has already stripped the
    /// stamp by the time it runs; this probe reads a tree that is still stamped, so it must ask
    /// for the same view explicitly. ~keep
    ///
    /// `--check` throughout: this writes nothing.
    pub(crate) fn probe(roots: &[PathBuf], base_dir: &Path) -> Self {
        if roots.is_empty() || !super::is_tool_available("poly") {
            return Self { scopes: None };
        }
        let scopes = roots
            .iter()
            .map(|root| {
                let root = normalize_path(root, base_dir);
                let mut args: Vec<String> = vec![
                    "fmt".to_owned(),
                    "--check".to_owned(),
                    "--format".to_owned(),
                    "json".to_owned(),
                    "--fix-generated".to_owned(),
                    root.to_string_lossy().into_owned(),
                ];
                push_poly_format_excludes(&mut args);
                let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
                let covered = match super::run_poly_capturing(&arg_refs, &root) {
                    Ok(stdout) => parse_poly_coverage(&stdout, &root),
                    Err(error) => {
                        tracing::debug!(
                            scope = %root.display(),
                            "poly coverage probe failed ({error:#}); drift check will not claim coverage in this scope"
                        );
                        None
                    }
                };
                PolyScope { root, covered }
            })
            .collect();
        Self { scopes: Some(scopes) }
    }

    /// An explicitly empty coverage set: poly reaches nothing. Test-only seam, so the
    /// "poly owns it" and "poly does not" branches are both provable without depending on
    /// whichever poly the host running the suite happens to have. ~keep
    #[cfg(test)]
    pub(crate) fn covering(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            scopes: Some(vec![PolyScope {
                root: PathBuf::new(),
                covered: Some(paths.into_iter().collect()),
            }]),
        }
    }

    #[cfg(test)]
    pub(crate) fn covering_scopes(scopes: impl IntoIterator<Item = (PathBuf, Vec<PathBuf>)>) -> Self {
        Self {
            scopes: Some(
                scopes
                    .into_iter()
                    .map(|(root, paths)| PolyScope {
                        root,
                        covered: Some(paths.into_iter().collect()),
                    })
                    .collect(),
            ),
        }
    }

    /// An unavailable probe -- see the type doc for why this is distinct from an empty one.
    #[cfg(test)]
    pub(crate) fn unavailable() -> Self {
        Self { scopes: None }
    }

    /// `Some(true)`/`Some(false)` once the probe has run; `None` when it could not.
    pub(crate) fn covers(&self, path: &Path) -> Option<bool> {
        self.scope_for(path)?
            .covered
            .as_ref()
            .map(|covered| covered.contains(path))
    }

    /// The working directory the writer uses when it asks poly to format `path`. This remains
    /// known even when that scope's coverage probe failed. ~keep
    pub(crate) fn format_context(&self, path: &Path) -> Option<&Path> {
        self.scope_for(path).map(|scope| scope.root.as_path())
    }

    fn scope_for(&self, path: &Path) -> Option<&PolyScope> {
        self.scopes
            .as_ref()?
            .iter()
            .filter(|scope| path.starts_with(&scope.root))
            .max_by_key(|scope| scope.root.components().count())
    }
}

/// Absolutise every path poly reported into a set, or `None` when the payload could not be read.
///
/// poly echoes back the root spelling it was given, so a root of `.` yields `./packages/x/y.md`
/// and a root of `/abs/dir` yields an absolute path. Comparing those strings to alef's own path
/// values without normalising is a silent miss: during this module's own development a
/// string-keyed lookup reported **0 of 197** candidate paths as poly-covered when the true answer
/// was 32, and the zero looked exactly like a real finding. Every entry is therefore resolved
/// against `base_dir` and normalised before it enters the set. ~keep
fn parse_poly_coverage(stdout: &str, base_dir: &Path) -> Option<HashSet<PathBuf>> {
    let payload: serde_json::Value = serde_json::from_str(stdout).ok()?;
    let results = payload.get("results")?.as_array()?;
    let mut covered = HashSet::new();
    for entry in results {
        // A `skipped` reason means poly declined this file (no engine for the type, ...). It is
        // reported inside `results` as well as in the top-level `skipped` list, and it is not
        // coverage. ~keep
        if entry.get("skipped").is_some_and(|reason| !reason.is_null()) {
            continue;
        }
        let Some(path) = entry.get("path").and_then(serde_json::Value::as_str) else {
            continue;
        };
        covered.insert(normalize_reported_path(path, base_dir));
    }
    Some(covered)
}

/// Resolve one path poly reported to the same absolute, `.`-free form alef uses internally.
fn normalize_reported_path(reported: &str, base_dir: &Path) -> PathBuf {
    normalize_path(Path::new(reported), base_dir)
}

fn normalize_path(raw: &Path, base_dir: &Path) -> PathBuf {
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        base_dir.join(raw)
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other),
        }
    }
    normalized
}

/// Which formatter alef's own pass applies at `path`.
///
/// Ordered most-specific first, because the categories genuinely nest: an `[crates.e2e.format]`
/// override replaces the poly pass for its whole language directory, and the residuals below it
/// run on files poly is separately excluded from. Consulting [`PolyCoverage`] last means a path
/// claimed by a tool alef runs itself is never mistaken for a poly-owned one just because poly
/// also happens to walk it.
pub(crate) fn formatting_owner(
    path: &Path,
    config: &ResolvedCrateConfig,
    base_dir: &Path,
    coverage: &PolyCoverage,
) -> FormattingOwner {
    if let Some(command) = e2e_override_for(path, config, base_dir) {
        return FormattingOwner::E2eOverride(command);
    }
    if let Some(residual) = residual_for(path, config, base_dir) {
        return FormattingOwner::Residual(residual);
    }
    match coverage.covers(path) {
        Some(true) => FormattingOwner::Poly,
        Some(false) => FormattingOwner::None,
        // Probe unavailable. Claiming `None` here would assert "the render is the final bytes"
        // on no evidence at all; `Poly` routes the caller to the real-formatter comparison,
        // which degrades to a plain content comparison by itself when poly turns out not to
        // touch the file. The conservative direction is the one that still examines something. ~keep
        None => FormattingOwner::Poly,
    }
}

/// The project-wide tool alef runs itself over `path`, if any.
///
/// `Cargo.toml` is claimed by `cargo sort -n -w`, which [`super::converge_full_regen`] runs at
/// the workspace root on every full regen. It is very likely a no-op on alef's own emitted
/// manifests -- the templates already emit dependencies in sorted order -- but "is a no-op
/// today" is a property of the emitter, not of the pipeline, and a reader that assumed it would
/// silently start lying the first time a template emitted an out-of-order key. Claiming it here
/// costs a handful of counted, disclosed skips and cannot rot. ~keep
fn residual_for(path: &Path, config: &ResolvedCrateConfig, base_dir: &Path) -> Option<&'static str> {
    let owned_by_zig = super::zig_format_roots(config, base_dir).into_iter().any(|root| {
        (path.starts_with(root.join("src")) && path.extension().and_then(|extension| extension.to_str()) == Some("zig"))
            || path == root.join("build.zig")
    });
    if owned_by_zig {
        return Some("zig fmt");
    }
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("ex" | "exs") => Some("mix format"),
        _ if path.file_name().is_some_and(|name| name == "Cargo.toml") => Some("cargo sort -n -w"),
        _ => None,
    }
}

/// The `[crates.e2e.format]` command that owns `path`, if one does.
///
/// A path belongs to language `lang` when it sits under `<output>/<lang>/`, for either e2e
/// output root -- the plain one and the registry-mode one (`[crates.e2e.registry] output`,
/// conventionally `test_apps/`), which is a second tree generated from the same `E2eConfig` and
/// formatted by the same `format_language`.
fn e2e_override_for(path: &Path, config: &ResolvedCrateConfig, base_dir: &Path) -> Option<String> {
    if let Some(e2e) = &config.e2e {
        for root in e2e_output_roots(e2e) {
            let root_path = base_dir.join(root);
            let Ok(relative) = path.strip_prefix(&root_path) else {
                continue;
            };
            let Some(language) = relative.components().next() else {
                continue;
            };
            let language = language.as_os_str().to_string_lossy();
            if let Some(command) = e2e.format.get(language.as_ref()) {
                return Some(command.clone());
            }
        }
    }
    None
}

/// Both output roots one `[crates.e2e]` block generates into.
///
/// Deliberately not [`crate::core::config::E2eConfig::effective_output`], which answers for the
/// ONE mode this process happens to be running in. A reader classifying a committed tree has to
/// speak for both: `alef all` generates the plain tree and the registry-mode tree in the same
/// run, from the same config, through the same `format_language`, so both are in scope on every
/// verify regardless of which mode the current process would pick. ~keep
fn e2e_output_roots(e2e: &crate::core::config::E2eConfig) -> [&str; 2] {
    [e2e.output.as_str(), e2e.registry.output.as_str()]
}

#[cfg(test)]
#[path = "owner_tests.rs"]
mod tests;
