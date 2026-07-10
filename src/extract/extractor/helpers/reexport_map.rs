use std::collections::HashSet;

use ahash::AHashMap;

use super::attributes::is_pub;

/// Represents what a `pub use` re-exports from a specific module.
#[derive(Debug)]
pub(crate) enum ReexportKind {
    /// `pub use module::*` — re-export everything
    Glob,
    /// `pub use module::{A, B}` — re-export specific names
    Names(HashSet<String>),
}

/// Collect pub use re-exports at the current module level, grouped by source module.
///
/// Returns a map from module name to the kind of re-export (glob or named).
/// Only tracks `pub use <ident>::...` where `<ident>` is not `self`/`super`/`crate`
/// (those are internal references handled elsewhere).
pub(crate) fn collect_reexport_map(items: &[syn::Item]) -> AHashMap<String, ReexportKind> {
    let mut map: AHashMap<String, ReexportKind> = AHashMap::new();
    for item in items {
        if let syn::Item::Use(item_use) = item {
            if is_pub(&item_use.vis) {
                collect_reexport_from_tree(&item_use.tree, &mut map);
            }
        }
    }
    map
}

/// Walk a use tree and populate the reexport map.
fn collect_reexport_from_tree(tree: &syn::UseTree, map: &mut AHashMap<String, ReexportKind>) {
    if let syn::UseTree::Path(use_path) = tree {
        let root_ident = use_path.ident.to_string();
        if root_ident == "self" {
            collect_reexport_from_tree(&use_path.tree, map);
            return;
        }
        if root_ident == "super" || root_ident == "crate" {
            return;
        }
        collect_reexport_leaves(&root_ident, &use_path.tree, map);
    } else if let syn::UseTree::Group(group) = tree {
        for item in &group.items {
            collect_reexport_from_tree(item, map);
        }
    }
}

/// Collect leaves from a use subtree rooted at a known module name.
fn collect_reexport_leaves(module: &str, tree: &syn::UseTree, map: &mut AHashMap<String, ReexportKind>) {
    match tree {
        syn::UseTree::Glob(_) => {
            map.insert(module.to_string(), ReexportKind::Glob);
        }
        syn::UseTree::Name(use_name) => {
            let name = use_name.ident.to_string();
            match map.get_mut(module) {
                Some(ReexportKind::Glob) => {}
                Some(ReexportKind::Names(names)) => {
                    names.insert(name);
                }
                None => {
                    let mut names = HashSet::new();
                    names.insert(name);
                    map.insert(module.to_string(), ReexportKind::Names(names));
                }
            }
        }
        syn::UseTree::Rename(use_rename) => {
            let name = use_rename.rename.to_string();
            match map.get_mut(module) {
                Some(ReexportKind::Glob) => {}
                Some(ReexportKind::Names(names)) => {
                    names.insert(name);
                }
                None => {
                    let mut names = HashSet::new();
                    names.insert(name);
                    map.insert(module.to_string(), ReexportKind::Names(names));
                }
            }
        }
        syn::UseTree::Path(use_path) => {
            collect_reexport_leaves(module, &use_path.tree, map);
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_reexport_leaves(module, item, map);
            }
        }
    }
}
