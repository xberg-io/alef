use std::path::{Path, PathBuf};

use alef_core::ir::ApiSurface;
use anyhow::Result;
use syn;

use super::helpers::ReexportKind;
use super::helpers::extract_cfg_condition;

/// Resolve a `pub use` tree, extracting re-exported items from workspace sibling crates.
pub(crate) fn resolve_use_tree(
    tree: &syn::UseTree,
    crate_name: &str,
    surface: &mut ApiSurface,
    workspace_root: Option<&Path>,
    visited: &mut Vec<PathBuf>,
    attrs: &[syn::Attribute],
) -> Result<()> {
    let cfg = extract_cfg_condition(attrs);
    match tree {
        syn::UseTree::Path(use_path) => {
            let root_ident = use_path.ident.to_string();

            // Skip self/super/crate references — already handled by mod resolution
            if root_ident == "self" || root_ident == "super" || root_ident == "crate" {
                return Ok(());
            }

            // This is an external crate reference like `use other_crate::...`
            resolve_external_use(
                &root_ident,
                &use_path.tree,
                crate_name,
                surface,
                workspace_root,
                visited,
                cfg,
            )
        }
        syn::UseTree::Group(group) => {
            for tree in &group.items {
                resolve_use_tree(tree, crate_name, surface, workspace_root, visited, attrs)?;
            }
            Ok(())
        }
        // `pub use something;` — a single ident, skip (not an external crate path)
        _ => Ok(()),
    }
}

/// Resolve `pub use external_crate::...` by finding the crate source and extracting named items.
fn resolve_external_use(
    ext_crate_name: &str,
    subtree: &syn::UseTree,
    crate_name: &str,
    surface: &mut ApiSurface,
    workspace_root: Option<&Path>,
    visited: &mut Vec<PathBuf>,
    cfg: Option<String>,
) -> Result<()> {
    let Some(crate_source) = find_crate_source(ext_crate_name, workspace_root) else {
        return Ok(());
    };

    let canonical = std::fs::canonicalize(&crate_source).unwrap_or_else(|_| crate_source.clone());
    if visited.contains(&canonical) {
        return Ok(());
    }
    // Push to visited BEFORE any recursive calls to prevent infinite cycles
    visited.push(canonical.clone());

    // Parse the external crate source
    let content = match std::fs::read_to_string(&crate_source) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };
    let file = match syn::parse_file(&content) {
        Ok(f) => f,
        Err(_) => return Ok(()),
    };

    // Extract the full surface of the external crate into a temporary surface
    let mut ext_surface = ApiSurface {
        crate_name: crate_name.to_string(), // Use our crate name for the rust_path
        version: String::new(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };
    let mut rwa = ahash::AHashSet::new();
    super::extract_items(
        &file.items,
        &canonical,
        crate_name,
        "",
        &mut ext_surface,
        workspace_root,
        visited,
        &mut rwa,
    )?;

    // Collect the names we want to import
    let filter = collect_use_names(subtree);

    match filter {
        UseFilter::All => {
            merge_surface(surface, ext_surface, cfg);
        }
        UseFilter::Names(names) => {
            merge_surface_filtered(surface, ext_surface, &names, cfg);
        }
    }

    Ok(())
}

/// What names does a use subtree import?
pub(crate) enum UseFilter {
    /// `use crate::*` — import everything
    All,
    /// `use crate::{A, B}` or `use crate::A` — import specific names
    Names(Vec<String>),
}

/// Collect the leaf names from a use subtree.
pub(crate) fn collect_use_names(tree: &syn::UseTree) -> UseFilter {
    match tree {
        syn::UseTree::Glob(_) => UseFilter::All,
        syn::UseTree::Name(name) => UseFilter::Names(vec![name.ident.to_string()]),
        syn::UseTree::Rename(rename) => UseFilter::Names(vec![rename.rename.to_string()]),
        syn::UseTree::Path(path) => collect_use_names(&path.tree),
        syn::UseTree::Group(group) => {
            let mut names = Vec::new();
            for item in &group.items {
                match collect_use_names(item) {
                    UseFilter::All => return UseFilter::All,
                    UseFilter::Names(n) => names.extend(n),
                }
            }
            UseFilter::Names(names)
        }
    }
}

/// Merge all items from `src` into `dst`, skipping duplicates.
pub(crate) fn merge_surface(dst: &mut ApiSurface, src: ApiSurface, cfg: Option<String>) {
    for mut ty in src.types {
        if !dst.types.iter().any(|t| t.name == ty.name) {
            if cfg.is_some() && ty.cfg.is_none() {
                ty.cfg = cfg.clone();
            }
            dst.types.push(ty);
        }
    }
    for mut func in src.functions {
        if !dst.functions.iter().any(|f| f.name == func.name) {
            if cfg.is_some() && func.cfg.is_none() {
                func.cfg = cfg.clone();
            }
            dst.functions.push(func);
        }
    }
    for mut en in src.enums {
        if !dst.enums.iter().any(|e| e.name == en.name) {
            if cfg.is_some() && en.cfg.is_none() {
                en.cfg = cfg.clone();
            }
            dst.enums.push(en);
        }
    }
}

/// Merge only items whose name is in `names` from `src` into `dst`.
pub(crate) fn merge_surface_filtered(dst: &mut ApiSurface, src: ApiSurface, names: &[String], cfg: Option<String>) {
    for mut ty in src.types {
        if names.contains(&ty.name) && !dst.types.iter().any(|t| t.name == ty.name) {
            if cfg.is_some() && ty.cfg.is_none() {
                ty.cfg = cfg.clone();
            }
            dst.types.push(ty);
        }
    }
    for mut func in src.functions {
        if names.contains(&func.name) && !dst.functions.iter().any(|f| f.name == func.name) {
            if cfg.is_some() && func.cfg.is_none() {
                func.cfg = cfg.clone();
            }
            dst.functions.push(func);
        }
    }
    for mut en in src.enums {
        if names.contains(&en.name) && !dst.enums.iter().any(|e| e.name == en.name) {
            if cfg.is_some() && en.cfg.is_none() {
                en.cfg = cfg.clone();
            }
            dst.enums.push(en);
        }
    }
}

/// Find the `src/lib.rs` of a workspace sibling crate.
pub(crate) fn find_crate_source(dep_crate_name: &str, workspace_root: Option<&Path>) -> Option<PathBuf> {
    let root = workspace_root?;

    // Read workspace Cargo.toml
    let cargo_toml = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let value: toml::Value = toml::from_str(&cargo_toml).ok()?;

    // Check [dependencies] for path deps
    if let Some(deps) = value.get("dependencies").and_then(|d| d.as_table()) {
        if let Some(path) = resolve_dep_path(deps, dep_crate_name, root) {
            return Some(path);
        }
    }

    // Check [workspace.dependencies]
    if let Some(deps) = value
        .get("workspace")
        .and_then(|w| w.get("dependencies"))
        .and_then(|d| d.as_table())
    {
        if let Some(path) = resolve_dep_path(deps, dep_crate_name, root) {
            return Some(path);
        }
    }

    // Heuristic: look for crates/{crate_name}/src/lib.rs
    let heuristic = root.join("crates").join(dep_crate_name).join("src/lib.rs");
    if heuristic.exists() {
        return Some(heuristic);
    }

    // Try with hyphens replaced by underscores and vice versa
    let alt_name = if dep_crate_name.contains('-') {
        dep_crate_name.replace('-', "_")
    } else {
        dep_crate_name.replace('_', "-")
    };
    let alt = root.join("crates").join(&alt_name).join("src/lib.rs");
    if alt.exists() {
        return Some(alt);
    }

    None
}

/// Try to resolve a dependency path from a TOML dependencies table.
fn resolve_dep_path(deps: &toml::map::Map<String, toml::Value>, dep_name: &str, root: &Path) -> Option<PathBuf> {
    let dep = deps.get(dep_name)?;
    let path = dep.get("path").and_then(|p| p.as_str())?;
    let crate_dir = root.join(path);
    let lib_rs = crate_dir.join("src/lib.rs");
    if lib_rs.exists() { Some(lib_rs) } else { None }
}

/// Extract a `mod` declaration and recursively process its contents.
#[allow(clippy::too_many_arguments)]
pub(crate) fn extract_module(
    item_mod: &syn::ItemMod,
    source_path: &Path,
    crate_name: &str,
    module_path: &str,
    reexport_map: &ahash::AHashMap<String, ReexportKind>,
    surface: &mut ApiSurface,
    workspace_root: Option<&Path>,
    visited: &mut Vec<PathBuf>,
) -> Result<()> {
    let mod_name = item_mod.ident.to_string();

    // Build the new module path for items inside this module.
    // If the parent has a glob re-export (`pub use mod_name::*`), all items from this
    // submodule are available at the parent level, so they keep the parent's module_path.
    let reexport_kind = reexport_map.get(&mod_name);
    let has_glob_reexport = matches!(reexport_kind, Some(ReexportKind::Glob));

    // For glob re-exports, items keep the parent's module_path (flattened).
    // For named re-exports, items get the deep path first, then we post-process.
    let new_module_path = if has_glob_reexport {
        module_path.to_string()
    } else if module_path.is_empty() {
        mod_name.clone()
    } else {
        format!("{module_path}::{mod_name}")
    };

    // Track surface sizes before extraction so we can post-process named re-exports.
    let named_reexports = match reexport_kind {
        Some(ReexportKind::Names(names)) => Some(names),
        _ => None,
    };
    let (types_before, enums_before, fns_before) = if named_reexports.is_some() {
        (surface.types.len(), surface.enums.len(), surface.functions.len())
    } else {
        (0, 0, 0)
    };

    // Inline module: `pub mod foo { ... }`
    let mut rwa = ahash::AHashSet::new();
    if let Some((_, items)) = &item_mod.content {
        super::extract_items(
            items,
            source_path,
            crate_name,
            &new_module_path,
            surface,
            workspace_root,
            visited,
            &mut rwa,
        )?;
    } else {
        // External module: `pub mod foo;` — resolve to file
        let parent_dir = source_path.parent().unwrap_or_else(|| Path::new("."));

        // Strip the `r#` raw-identifier prefix that `syn` includes in `Ident::to_string()`.
        // Raw identifiers like `mod r#trait;` refer to a file named `trait.rs`, not `r#trait.rs`.
        let file_name = mod_name.strip_prefix("r#").unwrap_or(&mod_name);

        // Try `<file_name>.rs` first, then `<file_name>/mod.rs`
        let candidates = [
            parent_dir.join(format!("{file_name}.rs")),
            parent_dir.join(file_name).join("mod.rs"),
        ];

        let mut found = false;
        for candidate in &candidates {
            if candidate.exists() {
                // Track this file as visited to prevent duplicate processing
                // when the same file appears both as a `pub mod` submodule and
                // as a top-level source in alef.toml.
                let canonical_candidate = std::fs::canonicalize(candidate).unwrap_or_else(|_| candidate.to_path_buf());
                if visited.contains(&canonical_candidate) {
                    found = true;
                    break;
                }
                visited.push(canonical_candidate);

                let content = std::fs::read_to_string(candidate)
                    .with_context(|| format!("Failed to read module file: {}", candidate.display()))?;
                let file = syn::parse_file(&content)
                    .with_context(|| format!("Failed to parse module file: {}", candidate.display()))?;
                let mut rwa2 = ahash::AHashSet::new();
                super::extract_items(
                    &file.items,
                    candidate,
                    crate_name,
                    &new_module_path,
                    surface,
                    workspace_root,
                    visited,
                    &mut rwa2,
                )?;
                found = true;
                break;
            }
        }

        if !found {
            return Ok(());
        }
    }

    // Post-process named re-exports: shorten rust_path for items whose names match.
    // Also prune items from private modules that are NOT in the re-export list,
    // since they can't be accessed from outside.
    if let Some(names) = named_reexports {
        let parent_prefix = if module_path.is_empty() {
            crate_name.to_string()
        } else {
            format!("{crate_name}::{module_path}")
        };

        for ty in &mut surface.types[types_before..] {
            if names.contains(&ty.name) {
                ty.rust_path = format!("{parent_prefix}::{}", ty.name);
            }
        }
        for en in &mut surface.enums[enums_before..] {
            if names.contains(&en.name) {
                en.rust_path = format!("{parent_prefix}::{}", en.name);
            }
        }
        for func in &mut surface.functions[fns_before..] {
            if names.contains(&func.name) {
                func.rust_path = format!("{parent_prefix}::{}", func.name);
            }
        }

        // Prune non-re-exported items from private modules.
        // Items in private modules are only accessible via their explicit re-exports.
        if !super::helpers::is_pub(&item_mod.vis) {
            let new_types: Vec<_> = surface.types.drain(types_before..).collect();
            surface
                .types
                .extend(new_types.into_iter().filter(|ty| names.contains(&ty.name)));
            let new_enums: Vec<_> = surface.enums.drain(enums_before..).collect();
            surface
                .enums
                .extend(new_enums.into_iter().filter(|en| names.contains(&en.name)));
            let new_fns: Vec<_> = surface.functions.drain(fns_before..).collect();
            surface
                .functions
                .extend(new_fns.into_iter().filter(|f| names.contains(&f.name)));
        }
    }

    Ok(())
}

use anyhow::Context;
