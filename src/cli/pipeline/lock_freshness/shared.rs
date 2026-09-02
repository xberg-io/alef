//! Helpers shared by two or more ecosystem-specific lock-freshness gates in this module family.
//!
//! Kept separate from any one ecosystem file on purpose: neither helper below names a language,
//! and folding either into `cargo.rs`/`node.rs`/etc. would make a future reader wonder whether it
//! is safe to change for that one ecosystem alone. See `avoid-duplication` -- this is the "single
//! reason to change" shared code the rule asks for, not a premature abstraction: both helpers were
//! already serving two call sites before this file existed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The exact `(name, requirement)` alef's own registry-mode `test_apps` generator would write
/// for `lang`'s self-dependency on this crate's own published package -- the uv/pnpm sibling of
/// the cargo path's `discover_cargo_locks`/`registry_dependencies_on_local_crates`.
///
/// Reuses [`crate::core::config::e2e::E2eConfig::resolve_package`] -- the exact function
/// [`crate::e2e::codegen::python::PythonE2eCodegen`] and
/// [`crate::e2e::codegen::typescript::TypeScriptCodegen`] call to resolve their own `pkg_name`/
/// `pkg_version` in `DependencyMode::Registry` -- rather than re-deriving the package identity
/// from `[python]`/`[node]` config (`python_pip_name`/`node_package_name`), which is a DIFFERENT
/// knob: `packages/python/pyproject.toml`'s own `[project] name` and
/// `test_apps/python/pyproject.toml`'s dependency name are independently configurable and are not
/// guaranteed to agree (the e2e/test_apps package can be renamed, repathed, or version-pinned
/// separately from the published package's own manifest). `normalize` mirrors each generator's
/// own requirement-text rendering (`python`: `normalize_python_version`, PEP 508 comparator
/// handling; `node`: passed through verbatim, matching `render_package_json`'s "never strip an
/// explicit range operator" comment) so the returned `requirement` is byte-identical to what the
/// generator itself would have written, not a semver-equivalent reconstruction.
///
/// Deliberately conservative, returning `None` (never an exemption) unless BOTH an identity (per
/// `identity`, typically `name`, but `module` for Go -- see below) and `version` are explicitly
/// set on `[crates.e2e.registry.packages.<lang>]` (falling back to the base
/// `[crates.e2e.packages.<lang>]` only as `resolve_package` itself already does): the real
/// generators have a further fallback of their own for an unset name (derived from the e2e call's
/// `module`) and version (`resolved_version()`, then `"0.1.0"`), but registry-mode test apps are
/// only ever meaningful with an explicit, publishable package identity in practice, and guessing
/// at that derived fallback here risks matching a name this check has no real authority over.
///
/// `identity` extracts the field `lang`'s ecosystem actually keys a dependency requirement on --
/// `name` for every ecosystem except Go, which has no `name` concept at all and instead pins by
/// `module` (a Go module path, e.g. `github.com/xberg-io/html-to-markdown/packages/go/v3`); the Go
/// caller passes `|package| package.name.clone().or_else(|| package.module.clone())` so a
/// `name`-based override still wins if one is ever configured. Threaded as a closure, the same
/// shape as `normalize`, rather than a Go-specific branch here, per this module's own doc: neither
/// helper should name a language.
///
/// Precise by construction for every caller: `lang` selects a single `[crates.e2e...packages.<lang>]`
/// row, so this can only ever match the requirement written for the crate actually being
/// processed -- unlike the cargo gate's own exemption (`super::cargo::explained_by_pending_publish`
/// equivalent lives in `version_lockfiles.rs`), which keys on any git-tracked in-tree package
/// sharing a name and version, this has no such collision surface to begin with. ~keep
pub(super) fn registry_self_dependency(
    resolved_cfg: &crate::core::config::ResolvedCrateConfig,
    lang: &str,
    identity: impl Fn(&crate::core::config::e2e::PackageRef) -> Option<String>,
    normalize: impl Fn(&str) -> String,
) -> Option<RegistrySelfDependency> {
    let mut e2e_config = resolved_cfg.e2e.clone()?;
    e2e_config.dep_mode = crate::core::config::e2e::DependencyMode::Registry;
    let package = e2e_config.resolve_package(lang)?;
    let version = package.version.clone()?;
    let name = identity(&package)?;
    Some(RegistrySelfDependency {
        name,
        requirement: normalize(&version),
    })
}

/// See [`registry_self_dependency`].
pub(super) struct RegistrySelfDependency {
    pub(super) name: String,
    pub(super) requirement: String,
}

/// Directories holding a `file_name` manifest that [`GeneratedFile::carries_alef_marker`] can
/// never certify, because the format has no comment syntax to carry an `alef:hash:` marker at all
/// (`generated_header: false` JSON, principally `package.json`). A lock-freshness gate keyed only
/// on this run's in-memory `current_gen_paths` -- itself filtered by that same marker predicate,
/// see [`super::super::generate::stampable_output_paths`] -- structurally never examines these paths, in
/// any run, which is the gap this function exists to close.
///
/// Reads the committed ownership record ([`crate::cli::cache::read_committed_owned_paths`],
/// `.alef-ownership.toml`) instead: it is the durable, general-purpose list of every path alef has
/// authorised itself to own *precisely because* it cannot carry a marker -- populated by
/// `write_scaffold_files_report`'s own write guard the first time it creates such a file, for
/// every unmarkable manifest kind alef emits, not only `package.json` for wasm. Filtering that
/// list by `file_name` extends `generated_paths` with every alef-managed manifest of that name
/// the registry already knows about, including one this particular run did not touch (a
/// `--crate`-scoped run, or a language skipped by the per-language cache) -- which is strictly
/// more correct for a freshness check than "only what this run happened to regenerate": the drift
/// this gate exists to catch does not require this run to have written the manifest, only for the
/// manifest and its sibling lock to disagree right now.
///
/// General by construction: nothing here names `wasm` or `node`. `crates/*-node/package.json` is
/// `generated_header: false` for the identical reason `crates/*-wasm/package.json` is and was
/// found to share this exact blind spot while auditing it, and both are closed by the same call
/// with no per-backend special case. `composer.json`, `Gemfile`, `go.mod`, and `pubspec.yaml` are
/// `generated_header: false` for the same underlying reason (JSON/Ruby-DSL/Go-module-file/YAML
/// package manifests, scaffolded once and thereafter user-owned per the
/// `generated-vs-user-maintained-boundary` context entry) and read from this identical list --
/// the PHP/Ruby/Go/Dart lock-freshness gates use this call exactly as the node gate always has,
/// with no new registration mechanism needed. ~keep
pub(super) fn registered_unmarkable_manifest_dirs(base_dir: &Path, file_name: &str) -> BTreeSet<PathBuf> {
    crate::cli::cache::read_committed_owned_paths(base_dir)
        .iter()
        .map(|relative| base_dir.join(relative))
        .filter(|path| path.file_name().and_then(|name| name.to_str()) == Some(file_name))
        .filter_map(|path| path.parent().map(Path::to_path_buf))
        .collect()
}
