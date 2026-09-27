use std::path::{Path, PathBuf};

use crate::core::ir::ApiSurface;

use super::helpers::{ReexportKind, collect_reexport_map, is_pub};
use super::reexports::{UseFilter, collect_use_names};

/// Resolve the directory that owns a module file's *children* -- the files backing any
/// `mod x;` declarations with no inline body found inside it.
///
/// For `mod.rs`/`lib.rs`/`main.rs`, children live directly beside the file (in its own parent
/// directory). For any other file -- a 2018-edition "sibling" module file such as `ssrf.rs` or
/// `config.rs` -- children live in a same-named subdirectory beside it (`ssrf/`, `config/`).
/// Every caller that resolves a `mod x;` declaration's file must go through this function:
/// using `source.parent()` directly is only correct for the `mod.rs`/`lib.rs` case, and silently
/// searches the wrong directory (one level too shallow) for a sibling-style file, missing the
/// module entirely. ~keep
pub(super) fn module_children_dir(source: &Path) -> PathBuf {
    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    match source.file_name().and_then(|n| n.to_str()) {
        Some("mod.rs") | Some("lib.rs") | Some("main.rs") => parent.to_path_buf(),
        _ => match source.file_stem().and_then(|s| s.to_str()) {
            Some(stem) if !stem.is_empty() => parent.join(stem),
            _ => parent.to_path_buf(),
        },
    }
}

/// Resolve the file that declares (and may carry a `pub use` re-exporting) the module whose own
/// files live in `module_dir` -- the inverse of [`module_children_dir`].
///
/// The declaring file is either `<module_dir>/mod.rs` (`lib.rs`/`main.rs` at the crate root), or
/// the 2018-edition sibling file `<module_dir's parent>/<module_dir's name>.rs` beside it.
fn find_module_declaring_file(module_dir: &Path) -> Option<PathBuf> {
    for candidate_name in ["mod.rs", "lib.rs", "main.rs"] {
        let candidate = module_dir.join(candidate_name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    let name = module_dir.file_name()?.to_str()?;
    let sibling = module_dir.parent()?.join(format!("{name}.rs"));
    if sibling.exists() { Some(sibling) } else { None }
}

/// Apply named re-export shortening from the parent module file.
///
/// When a source file like `cache/core.rs` produces items with paths like
/// `sample_core::cache::core::GenericCache`, and the parent `cache/mod.rs` has
/// `pub use core::{GenericCache, ...}`, this shortens the path to
/// `sample_core::cache::GenericCache`.
///
/// Finds the parent's declaring file via [`find_module_declaring_file`], so a parent module
/// written in 2018-edition sibling-file style (`types/config.rs` owning `types/config/`) is
/// found exactly like a `mod.rs`-style parent -- not just the `<parent_dir>/mod.rs` and
/// `<parent_dir>/lib.rs` cases this function originally special-cased. ~keep
pub(super) fn apply_parent_reexport_shortening(
    source: &Path,
    crate_name: &str,
    module_path: &str,
    surface: &mut ApiSurface,
    types_before: usize,
    enums_before: usize,
    fns_before: usize,
) {
    let parent_dir = match source.parent() {
        Some(p) => p,
        None => return,
    };

    let parent_content = match find_module_declaring_file(parent_dir) {
        Some(declaring_file) if declaring_file != source => std::fs::read_to_string(&declaring_file).ok(),
        _ => None,
    };

    let Some(content) = parent_content else {
        return;
    };

    let Ok(parent_file) = syn::parse_file(&content) else {
        return;
    };

    let mod_name = source.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if mod_name.is_empty() || mod_name == "mod" {
        return;
    }

    let reexport_map = collect_reexport_map(&parent_file.items);

    let mut reexported_names = std::collections::HashSet::new();
    for item in &parent_file.items {
        if let syn::Item::Use(item_use) = item
            && is_pub(&item_use.vis)
            && let syn::UseTree::Path(use_path) = &item_use.tree
            && use_path.ident == mod_name
        {
            match collect_use_names(&use_path.tree) {
                UseFilter::All => {
                    let parent_module_path = module_path.rsplit_once("::").map(|(p, _)| p).unwrap_or("");
                    let parent_prefix = if parent_module_path.is_empty() {
                        crate_name.to_string()
                    } else {
                        format!("{crate_name}::{parent_module_path}")
                    };
                    for ty in &mut surface.types[types_before..] {
                        ty.rust_path = format!("{parent_prefix}::{}", ty.name);
                    }
                    for en in &mut surface.enums[enums_before..] {
                        en.rust_path = format!("{parent_prefix}::{}", en.name);
                    }
                    for func in &mut surface.functions[fns_before..] {
                        func.rust_path = format!("{parent_prefix}::{}", func.name);
                    }
                    return;
                }
                UseFilter::Names(names) => {
                    reexported_names.extend(names);
                }
            }
        }
    }

    if let Some(ReexportKind::Names(names)) = reexport_map.get(mod_name) {
        reexported_names.extend(names.iter().cloned());
    } else if matches!(reexport_map.get(mod_name), Some(ReexportKind::Glob)) {
        let parent_module_path = module_path.rsplit_once("::").map(|(p, _)| p).unwrap_or("");
        let parent_prefix = if parent_module_path.is_empty() {
            crate_name.to_string()
        } else {
            format!("{crate_name}::{parent_module_path}")
        };
        for ty in &mut surface.types[types_before..] {
            ty.rust_path = format!("{parent_prefix}::{}", ty.name);
        }
        for en in &mut surface.enums[enums_before..] {
            en.rust_path = format!("{parent_prefix}::{}", en.name);
        }
        for func in &mut surface.functions[fns_before..] {
            func.rust_path = format!("{parent_prefix}::{}", func.name);
        }
        return;
    }

    if reexported_names.is_empty() {
        return;
    }

    let parent_module_path = module_path.rsplit_once("::").map(|(p, _)| p).unwrap_or("");
    let parent_prefix = if parent_module_path.is_empty() {
        crate_name.to_string()
    } else {
        format!("{crate_name}::{parent_module_path}")
    };

    for ty in &mut surface.types[types_before..] {
        if reexported_names.contains(&ty.name) {
            ty.rust_path = format!("{parent_prefix}::{}", ty.name);
        }
    }
    for en in &mut surface.enums[enums_before..] {
        if reexported_names.contains(&en.name) {
            en.rust_path = format!("{parent_prefix}::{}", en.name);
        }
    }
    for func in &mut surface.functions[fns_before..] {
        if reexported_names.contains(&func.name) {
            func.rust_path = format!("{parent_prefix}::{}", func.name);
        }
    }
}

/// Derive the module path from a source file's location relative to the crate source root.
///
/// For `lib.rs` (the root), returns `""`.
/// For `src/cache/core.rs` relative to `src/`, returns `"cache::core"`.
/// For `src/types/mod.rs` relative to `src/`, returns `"types"`.
/// Falls back to `""` if the path can't be derived (e.g. file is outside the crate tree).
pub(super) fn derive_module_path(source: &Path, crate_src_dir: Option<&Path>) -> String {
    let Some(root) = crate_src_dir else {
        return String::new();
    };

    let root_canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let source_canonical = std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());

    let Ok(relative) = source_canonical.strip_prefix(&root_canonical) else {
        return String::new();
    };

    let mut segments = Vec::new();
    for component in relative.iter() {
        let s = component.to_string_lossy();
        if s == "lib.rs" || s == "main.rs" {
            return String::new();
        } else if s == "mod.rs" {
            continue;
        } else if let Some(stem) = s.strip_suffix(".rs") {
            segments.push(stem.to_string());
        } else {
            segments.push(s.to_string());
        }
    }

    segments.join("::")
}

/// Reject any extracted item whose final `rust_path` still runs through a module the crate
/// itself declared without `pub` (issue #466: alef must never hand a backend an unreachable
/// path such as `crate::config::primitives::Thing` when `config` is a private module -- the
/// generated FFI call site cannot name it and the consumer's build fails with `E0433`).
///
/// `private_module_paths` is the full set of `crate`-relative module paths (e.g.
/// `"types::config"`) that were walked only because *some* ancestor's `pub use` bridged them --
/// every module in this set is still private in its own right. The named-reexport shortening in
/// [`super::reexports::extract_module`] is expected to rewrite every bridged item's `rust_path`
/// past every such segment before extraction finishes; this is the safety net for the case where
/// it could not -- e.g. a multi-hop `pub use a::b::c::Thing;` whose intermediate segment `b`
/// carries no re-export of its own, so no rewrite ever fires and the item's `rust_path` still
/// contains the private prefix. Fails loudly with every offending path named, rather than
/// silently shipping a path nothing outside the crate can spell. ~keep
pub(super) fn validate_no_private_path_leaks(
    surface: &ApiSurface,
    crate_name: &str,
    private_module_paths: &ahash::AHashSet<String>,
) -> anyhow::Result<()> {
    if private_module_paths.is_empty() {
        return Ok(());
    }

    let mut offenders: Vec<String> = Vec::new();
    let crate_prefix = format!("{crate_name}::");

    let mut check = |rust_path: &str| {
        let Some(rest) = rust_path.strip_prefix(&crate_prefix) else {
            return;
        };
        let segments: Vec<&str> = rest.split("::").collect();
        // The last segment is the item's own name, never a module -- only proper prefixes of
        // the module path can match a recorded private module. ~keep
        for end in 1..segments.len() {
            let prefix = segments[..end].join("::");
            if private_module_paths.contains(&prefix) {
                offenders.push(format!("{rust_path} (via private module `{crate_name}::{prefix}`)"));
                return;
            }
        }
    };

    for ty in &surface.types {
        check(&ty.rust_path);
    }
    for en in &surface.enums {
        check(&en.rust_path);
    }
    for func in &surface.functions {
        check(&func.rust_path);
    }

    if offenders.is_empty() {
        return Ok(());
    }

    offenders.sort();
    anyhow::bail!(
        "alef: unreachable private-module path(s) in extracted API surface -- these items are \
         only reachable through a module the crate declared without `pub`, and no bridging \
         `pub use` could be resolved all the way to a public path. Emitting them would produce \
         a generated binding that cannot compile (rustc E0433). Re-export the item directly from \
         every module on its path, or exclude it explicitly:\n  {}",
        offenders.join("\n  ")
    );
}
