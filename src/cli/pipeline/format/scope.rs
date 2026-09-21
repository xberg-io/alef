//! Which directories (and, for a workspace-root file no language owns, which individual paths)
//! a partial regen's format pass must reach.
//!
//! Split out of `format.rs` for the same file-modularization cap `bin_cli/core_commands.rs`
//! split `generate.rs` out for. ~keep

use crate::core::config::{Language, OutputLayout, ResolvedCrateConfig};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

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
pub(crate) fn poly_paths(
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

/// The complement of [`languages_owning_changed_paths`]: which of `changed_paths` lies inside
/// NO configured language's `package_dir`/`output_for`/crate-root directory at all -- a
/// workspace-root scaffold file like `.cargo/config.toml`, owned by no single language and
/// therefore invisible to [`poly_paths`]' partial-regen scope no matter which languages
/// `format_scope` names.
///
/// `alef generate`'s partial-regen format pass is built entirely from per-language
/// directories (see [`poly_paths`]'s own doc comment on why format scope must equal stamp
/// scope), so a file this function returns would otherwise never reach the formatter on that
/// path -- only `alef all`'s whole-tree convergence pass (`only_languages = None`) covers it.
/// The caller hands these back to poly directly, as individual file paths rather than a
/// directory, so a partial regen's format pass stays scoped to what changed instead of
/// widening into a full-tree walk. ~keep
pub(crate) fn unowned_changed_paths(
    config: &ResolvedCrateConfig,
    base_dir: &Path,
    languages: &[Language],
    changed_paths: &HashSet<PathBuf>,
) -> Vec<PathBuf> {
    if changed_paths.is_empty() {
        return Vec::new();
    }
    if languages.is_empty() {
        return changed_paths.iter().cloned().collect();
    }
    let full: HashSet<Language> = languages.iter().copied().collect();
    let dirs = poly_paths(config, base_dir, Some(&full), languages);
    changed_paths
        .iter()
        .filter(|path| !dirs.iter().any(|dir| path.starts_with(dir)))
        .cloned()
        .collect()
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
