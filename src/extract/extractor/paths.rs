use std::path::Path;

use crate::core::ir::ApiSurface;

use super::helpers::{ReexportKind, collect_reexport_map, is_pub};
use super::reexports::{UseFilter, collect_use_names};

/// Apply named re-export shortening from the parent module file.
///
/// When a source file like `cache/core.rs` produces items with paths like
/// `sample_core::cache::core::GenericCache`, and the parent `cache/mod.rs` has
/// `pub use core::{GenericCache, ...}`, this shortens the path to
/// `sample_core::cache::GenericCache`.
pub(super) fn apply_parent_reexport_shortening(
    source: &Path,
    crate_name: &str,
    module_path: &str,
    surface: &mut ApiSurface,
    types_before: usize,
    enums_before: usize,
    fns_before: usize,
) {
    // Find the parent module file (mod.rs in parent directory, or parent.rs)
    let parent_dir = match source.parent() {
        Some(p) => p,
        None => return,
    };

    // Check if there's a mod.rs in the same directory (for files like cache/core.rs,
    // the parent module is cache/mod.rs)
    let parent_mod = parent_dir.join("mod.rs");
    let parent_lib = parent_dir.join("lib.rs");
    let parent_content = if parent_mod.exists() && parent_mod != source {
        std::fs::read_to_string(&parent_mod).ok()
    } else if parent_lib.exists() && parent_lib != source {
        std::fs::read_to_string(&parent_lib).ok()
    } else {
        None
    };

    let Some(content) = parent_content else {
        return;
    };

    let Ok(parent_file) = syn::parse_file(&content) else {
        return;
    };

    // Get the module name of the source file (e.g., "core" for cache/core.rs)
    let mod_name = source.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if mod_name.is_empty() || mod_name == "mod" {
        return;
    }

    // Collect re-exports from the parent module
    let reexport_map = collect_reexport_map(&parent_file.items);

    // Also check for `pub use mod_name::{A, B}` statements directly
    let mut reexported_names = std::collections::HashSet::new();
    for item in &parent_file.items {
        if let syn::Item::Use(item_use) = item {
            if is_pub(&item_use.vis) {
                if let syn::UseTree::Path(use_path) = &item_use.tree {
                    if use_path.ident == mod_name {
                        match collect_use_names(&use_path.tree) {
                            UseFilter::All => {
                                // Glob re-export — all items are re-exported
                                // Shorten all items to parent path
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
            }
        }
    }

    // Also include names from the reexport_map
    if let Some(ReexportKind::Names(names)) = reexport_map.get(mod_name) {
        reexported_names.extend(names.iter().cloned());
    } else if matches!(reexport_map.get(mod_name), Some(ReexportKind::Glob)) {
        // Glob — shorten all
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

    // Apply shortening for named re-exports
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

    // Canonicalize both paths for reliable comparison
    let root_canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let source_canonical = std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());

    let Ok(relative) = source_canonical.strip_prefix(&root_canonical) else {
        return String::new();
    };

    // Convert path components to module segments.
    // `lib.rs` → "" (root), `cache/core.rs` → "cache::core", `types/mod.rs` → "types"
    let mut segments = Vec::new();
    for component in relative.iter() {
        let s = component.to_string_lossy();
        if s == "lib.rs" || s == "main.rs" {
            // Root file — no module path
            return String::new();
        } else if s == "mod.rs" {
            // mod.rs doesn't add a segment (the parent directory is the module name)
            continue;
        } else if let Some(stem) = s.strip_suffix(".rs") {
            segments.push(stem.to_string());
        } else {
            // Directory component
            segments.push(s.to_string());
        }
    }

    segments.join("::")
}
