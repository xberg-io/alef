//! Resolve which `Result` type alias a module has in scope.
//!
//! A crate may declare several `Result` aliases — a canonical one next to the crate error plus
//! private ones inside individual subsystems. Which alias a given module means when it writes
//! `Result<T>` is decided by that module's `use` statements, so error-type resolution has to read
//! them rather than assume a single crate-wide alias.

use std::cell::RefCell;

use ahash::AHashSet;

use crate::extract::type_resolver::ResultAliasScope;

thread_local! {
    /// The module whose items are being extracted right now, as far as `Result` path resolution
    /// is concerned. Needed to resolve `self::`/`super::` qualifications and Rust 2018 uniform
    /// paths in a return type, neither of which can be read off the type alone. ~keep
    static MODULE_CONTEXT: RefCell<Option<ModuleContext>> = const { RefCell::new(None) };
}

/// Where a module sits and which child modules it declares.
#[derive(Debug, Clone, Default)]
struct ModuleContext {
    module_path: String,
    local_modules: AHashSet<String>,
}

/// Installs the module context used to resolve qualified `Result` paths, restoring the enclosing
/// module's context on drop.
pub(crate) struct ResultModuleContextGuard(Option<ModuleContext>);

impl ResultModuleContextGuard {
    pub(crate) fn module_path() -> Option<String> {
        MODULE_CONTEXT.with(|context| context.borrow().as_ref().map(|context| context.module_path.clone()))
    }

    pub(crate) fn enter_child(items: &[syn::Item], name: &str) -> Self {
        let parent = Self::module_path().unwrap_or_default();
        let path = if parent.is_empty() {
            name.to_string()
        } else {
            format!("{parent}::{name}")
        };
        Self::enter(items, &path)
    }

    /// Enter the module at `module_path` whose items are `items`.
    pub(crate) fn enter(items: &[syn::Item], module_path: &str) -> Self {
        let context = ModuleContext {
            module_path: module_path.to_string(),
            local_modules: collect_local_module_names(items),
        };
        Self(MODULE_CONTEXT.with(|c| c.replace(Some(context))))
    }
}

impl Drop for ResultModuleContextGuard {
    fn drop(&mut self) {
        MODULE_CONTEXT.with(|c| {
            *c.borrow_mut() = self.0.take();
        });
    }
}

/// Which alias a *qualified* `Result` path in a return type names.
///
/// `crate::Result<T>` and `super::error::Result<T>` say outright which alias they mean, and that
/// beats whatever the module's `use` statements bring into scope under the bare name — qualifying
/// is usually done precisely because the bare name is already taken by another crate's `Result`.
/// Returns `None` for an unqualified `Result<T>`, whose meaning the module's imports do decide.
pub(crate) fn scope_for_result_type_path(path: &syn::Path) -> Option<ResultAliasScope> {
    if path.segments.len() < 2 {
        return None;
    }
    let mut prefix: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    prefix.pop();
    // A leading `::` always starts at a crate root, so it can never mean a local module. ~keep
    if path.leading_colon.is_some() && prefix.first().map(String::as_str) != Some("crate") {
        return Some(ResultAliasScope::Foreign);
    }
    MODULE_CONTEXT.with(|c| {
        let borrowed = c.borrow();
        let fallback = ModuleContext::default();
        let context = borrowed.as_ref().unwrap_or(&fallback);
        Some(scope_for_prefix(&prefix, &context.module_path, &context.local_modules))
    })
}

/// Resolve explicit imports before consulting this module's preindexed glob namespace.
///
/// A glob may export no `Result`, so it must not fall back to an unrelated crate-wide alias.
pub(crate) fn resolve_result_alias_scope(items: &[syn::Item], module_path: &str) -> Option<ResultAliasScope> {
    resolve_named_result_alias_scope(items, module_path).or_else(|| {
        (!collect_result_glob_scopes(items, module_path).is_empty())
            .then(|| ResultAliasScope::Crate(module_path.to_string()))
    })
}

pub(crate) fn resolve_named_result_alias_scope(items: &[syn::Item], module_path: &str) -> Option<ResultAliasScope> {
    let local_modules = collect_local_module_names(items);
    let mut prefix: Vec<String> = Vec::new();
    items.iter().find_map(|item| match item {
        syn::Item::Use(item_use) if !super::is_test_gated(&item_use.attrs) => {
            let scope = scope_from_tree(&item_use.tree, module_path, &local_modules, &mut prefix);
            scope.map(|scope| {
                if item_use.leading_colon.is_some() {
                    ResultAliasScope::Foreign
                } else {
                    scope
                }
            })
        }
        _ => None,
    })
}

pub(crate) fn result_alias_target_scope(
    ty: &syn::Type,
    items: &[syn::Item],
    module_path: &str,
) -> Option<ResultAliasScope> {
    let syn::Type::Path(type_path) = ty else { return None };
    let mut prefix = type_path
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    if prefix.pop().as_deref() != Some("Result") || prefix.is_empty() {
        return None;
    }
    if type_path.path.leading_colon.is_some() {
        return Some(ResultAliasScope::Foreign);
    }
    Some(scope_for_prefix(
        &prefix,
        module_path,
        &collect_local_module_names(items),
    ))
}

pub(crate) fn collect_result_glob_scopes(items: &[syn::Item], module_path: &str) -> Vec<ResultAliasScope> {
    let context = ModuleContext {
        module_path: module_path.to_string(),
        local_modules: collect_local_module_names(items),
    };
    let mut scopes = Vec::new();
    let mut prefix = Vec::new();
    for item in items {
        if let syn::Item::Use(item_use) = item
            && !super::is_test_gated(&item_use.attrs)
        {
            collect_glob_tree(
                &item_use.tree,
                &context,
                &mut prefix,
                &mut scopes,
                item_use.leading_colon.is_some(),
            );
        }
    }
    scopes
}

fn collect_glob_tree(
    tree: &syn::UseTree,
    context: &ModuleContext,
    prefix: &mut Vec<String>,
    scopes: &mut Vec<ResultAliasScope>,
    absolute: bool,
) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_glob_tree(&path.tree, context, prefix, scopes, absolute);
            prefix.pop();
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_glob_tree(item, context, prefix, scopes, absolute);
            }
        }
        syn::UseTree::Glob(_) => {
            let scope = if absolute {
                ResultAliasScope::Foreign
            } else {
                scope_for_prefix(prefix, &context.module_path, &context.local_modules)
            };
            scopes.push(scope);
        }
        _ => {}
    }
}

/// Names of the child modules declared in `items`.
///
/// Rust 2018 uniform paths let a `use` start at a module of the current module
/// (`pub use error::Result;` in `lib.rs`), which is otherwise indistinguishable from a foreign
/// crate name. Knowing the local module names is what tells the two apart. ~keep
fn collect_local_module_names(items: &[syn::Item]) -> AHashSet<String> {
    items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Mod(item_mod) => Some(item_mod.ident.to_string()),
            _ => None,
        })
        .collect()
}

/// Walk a use tree, accumulating the module path segments that precede each leaf.
fn scope_from_tree(
    tree: &syn::UseTree,
    module_path: &str,
    local_modules: &AHashSet<String>,
    prefix: &mut Vec<String>,
) -> Option<ResultAliasScope> {
    match tree {
        syn::UseTree::Path(use_path) => {
            prefix.push(use_path.ident.to_string());
            let found = scope_from_tree(&use_path.tree, module_path, local_modules, prefix);
            prefix.pop();
            found
        }
        syn::UseTree::Group(group) => group
            .items
            .iter()
            .find_map(|item| scope_from_tree(item, module_path, local_modules, prefix)),
        // The name the module actually writes is the imported ident, or the rename when there is
        // one — `use anyhow::Result as AnyResult` does not put `Result` in scope. ~keep
        syn::UseTree::Name(use_name) if use_name.ident == "Result" => {
            Some(scope_for_prefix(prefix, module_path, local_modules))
        }
        syn::UseTree::Rename(use_rename) if use_rename.rename == "Result" => {
            Some(scope_for_prefix(prefix, module_path, local_modules))
        }
        _ => None,
    }
}

/// Turn the path segments preceding an imported `Result` into the module that declares it.
///
/// `crate`/`self`/`super` prefixes and uniform paths into a locally declared module resolve
/// against `module_path`; anything else names another crate, whose `Result` carries its own error
/// type and must not borrow this crate's.
fn scope_for_prefix(prefix: &[String], module_path: &str, local_modules: &AHashSet<String>) -> ResultAliasScope {
    let segments: Vec<&str> = prefix.iter().map(String::as_str).collect();
    let mut index = 0;
    let mut base: Vec<&str> = Vec::new();

    match segments.first().copied() {
        Some("crate") => index = 1,
        Some("self") => {
            index = 1;
            base = split_module_path(module_path);
        }
        Some("super") => {
            base = split_module_path(module_path);
            while segments.get(index).copied() == Some("super") {
                index += 1;
                base.pop();
            }
        }
        Some(first) if local_modules.contains(first) => base = split_module_path(module_path),
        _ => return ResultAliasScope::Foreign,
    }

    base.extend(segments[index..].iter().copied());
    ResultAliasScope::Crate(base.join("::"))
}

/// Split a module path into segments (the crate root is the empty path).
fn split_module_path(module_path: &str) -> Vec<&str> {
    if module_path.is_empty() {
        Vec::new()
    } else {
        module_path.split("::").collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(source: &str) -> Vec<syn::Item> {
        syn::parse_str::<syn::File>(source)
            .expect("test source must parse")
            .items
    }

    #[test]
    fn crate_root_import_resolves_to_the_root_module() {
        let parsed = items("use crate::Result;");
        assert_eq!(
            resolve_result_alias_scope(&parsed, "plugins::embedding"),
            Some(ResultAliasScope::Crate(String::new()))
        );
    }

    #[test]
    fn qualified_crate_import_resolves_to_the_declaring_module() {
        let parsed = items("use crate::error::Result;");
        assert_eq!(
            resolve_result_alias_scope(&parsed, "plugins"),
            Some(ResultAliasScope::Crate("error".to_string()))
        );
    }

    #[test]
    fn super_import_resolves_against_the_parent_module() {
        let parsed = items("use super::error::Result;");
        assert_eq!(
            resolve_result_alias_scope(&parsed, "extraction::binary::model"),
            Some(ResultAliasScope::Crate("extraction::binary::error".to_string()))
        );
    }

    #[test]
    fn grouped_super_import_resolves_to_the_parent_module() {
        let parsed = items("use super::{ConversionResult, Result};");
        assert_eq!(
            resolve_result_alias_scope(&parsed, "convert_api"),
            Some(ResultAliasScope::Crate(String::new()))
        );
    }

    #[test]
    fn uniform_path_into_a_locally_declared_module_is_crate_local() {
        let parsed = items("pub mod error;\npub use error::{Result, SampleCrateError};");
        assert_eq!(
            resolve_result_alias_scope(&parsed, ""),
            Some(ResultAliasScope::Crate("error".to_string()))
        );
    }

    #[test]
    fn foreign_crate_import_is_not_a_crate_local_alias() {
        let parsed = items("use anyhow::Result;");
        assert_eq!(
            resolve_result_alias_scope(&parsed, "plugins"),
            Some(ResultAliasScope::Foreign)
        );
    }

    #[test]
    fn renamed_foreign_import_does_not_bring_result_into_scope() {
        let parsed = items("use anyhow::Result as AnyResult;");
        assert_eq!(resolve_result_alias_scope(&parsed, "plugins"), None);
    }

    #[test]
    fn module_without_a_result_import_has_no_scope() {
        let parsed = items("use std::sync::Arc;");
        assert_eq!(resolve_result_alias_scope(&parsed, "plugins"), None);
    }
}
