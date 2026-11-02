//! Resolve which `Result` type alias a module has in scope.
//!
//! A crate may declare several `Result` aliases — a canonical one next to the crate error plus
//! private ones inside individual subsystems. Which alias a given module means when it writes
//! `Result<T>` is decided by that module's `use` statements, so error-type resolution has to read
//! them rather than assume a single crate-wide alias.

use std::cell::RefCell;

use ahash::{AHashMap, AHashSet};

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
    local_namespaces: AHashSet<String>,
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
            local_namespaces: collect_local_namespace_names(items),
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
        Some(scope_for_prefix(
            &prefix,
            &context.module_path,
            &context.local_namespaces,
        ))
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

#[derive(Clone)]
pub(crate) struct ResultAliasImport {
    pub scope: ResultAliasScope,
    pub visible_from: String,
}

struct UseImport {
    name: Option<String>,
    path: Vec<String>,
    visibility: syn::Visibility,
    absolute: bool,
}

pub(crate) fn resolve_named_result_alias_scope(items: &[syn::Item], module_path: &str) -> Option<ResultAliasScope> {
    resolve_named_result_alias_import(items, module_path).map(|import| import.scope)
}

pub(crate) fn resolve_named_result_alias_import(items: &[syn::Item], module_path: &str) -> Option<ResultAliasImport> {
    let namespaces = collect_local_namespace_names(items);
    collect_use_imports(items).into_iter().find_map(|import| {
        (import.name.as_deref() == Some("Result")).then(|| ResultAliasImport {
            scope: import_scope(&import, &import.path[..import.path.len() - 1], module_path, &namespaces),
            visible_from: visibility_scope(&import.visibility, module_path),
        })
    })
}

pub(crate) fn result_alias_target_scope(
    ty: &syn::Type,
    items: &[syn::Item],
    module_path: &str,
) -> Option<ResultAliasScope> {
    let syn::Type::Path(type_path) = ty else { return None };
    let prefix = type_path
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    if type_path.path.leading_colon.is_some() {
        return Some(ResultAliasScope::Foreign);
    }
    Some(scope_for_prefix(
        &prefix,
        module_path,
        &collect_local_namespace_names(items),
    ))
}

pub(crate) fn collect_result_glob_scopes(items: &[syn::Item], module_path: &str) -> Vec<ResultAliasImport> {
    let namespaces = collect_local_namespace_names(items);
    collect_use_imports(items)
        .into_iter()
        .filter(|import| import.name.is_none())
        .map(|import| ResultAliasImport {
            scope: import_scope(&import, &import.path, module_path, &namespaces),
            visible_from: visibility_scope(&import.visibility, module_path),
        })
        .collect()
}

pub(crate) fn collect_result_module_imports(
    items: &[syn::Item],
    module_path: &str,
) -> AHashMap<String, ResultAliasScope> {
    let namespaces = collect_local_namespace_names(items);
    collect_use_imports(items)
        .into_iter()
        .filter_map(|import| {
            let name = import.name.as_ref()?;
            let path = if module_path.is_empty() {
                name.clone()
            } else {
                format!("{module_path}::{name}")
            };
            Some((path, import_scope(&import, &import.path, module_path, &namespaces)))
        })
        .collect()
}

pub(crate) fn visibility_scope(visibility: &syn::Visibility, module_path: &str) -> String {
    match visibility {
        syn::Visibility::Public(_) => String::new(),
        syn::Visibility::Inherited => module_path.to_string(),
        syn::Visibility::Restricted(restricted) => {
            let prefix = restricted
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>();
            match scope_for_prefix(&prefix, module_path, &AHashSet::new()) {
                ResultAliasScope::Crate(scope) => scope,
                ResultAliasScope::Foreign => module_path.to_string(),
            }
        }
    }
}

fn import_scope(
    import: &UseImport,
    prefix: &[String],
    module_path: &str,
    namespaces: &AHashSet<String>,
) -> ResultAliasScope {
    if import.absolute {
        ResultAliasScope::Foreign
    } else {
        scope_for_prefix(prefix, module_path, namespaces)
    }
}

fn collect_local_namespace_names(items: &[syn::Item]) -> AHashSet<String> {
    let mut names = items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Mod(module) => Some(module.ident.to_string()),
            _ => None,
        })
        .collect::<AHashSet<_>>();
    names.extend(collect_use_imports(items).into_iter().filter_map(|import| import.name));
    names
}

fn collect_use_imports(items: &[syn::Item]) -> Vec<UseImport> {
    let mut imports = Vec::new();
    for item in items {
        if let syn::Item::Use(item_use) = item
            && !super::is_test_gated(&item_use.attrs)
        {
            collect_use_tree(&item_use.tree, &mut Vec::new(), item_use, &mut imports);
        }
    }
    imports
}

fn collect_use_tree(tree: &syn::UseTree, prefix: &mut Vec<String>, item: &syn::ItemUse, imports: &mut Vec<UseImport>) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_use_tree(&path.tree, prefix, item, imports);
            prefix.pop();
        }
        syn::UseTree::Group(group) => {
            for child in &group.items {
                collect_use_tree(child, prefix, item, imports);
            }
        }
        syn::UseTree::Glob(_) => imports.push(UseImport {
            name: None,
            path: prefix.clone(),
            visibility: item.vis.clone(),
            absolute: item.leading_colon.is_some(),
        }),
        syn::UseTree::Name(name) => push_named_import(prefix, &name.ident.to_string(), None, item, imports),
        syn::UseTree::Rename(rename) => {
            push_named_import(
                prefix,
                &rename.ident.to_string(),
                Some(rename.rename.to_string()),
                item,
                imports,
            );
        }
    }
}

fn push_named_import(
    prefix: &[String],
    original: &str,
    rename: Option<String>,
    item: &syn::ItemUse,
    imports: &mut Vec<UseImport>,
) {
    let mut path = prefix.to_vec();
    let name = if original == "self" {
        let Some(name) = prefix.last() else { return };
        rename.unwrap_or_else(|| name.clone())
    } else {
        path.push(original.to_string());
        rename.unwrap_or_else(|| original.to_string())
    };
    imports.push(UseImport {
        name: Some(name),
        path,
        visibility: item.vis.clone(),
        absolute: item.leading_colon.is_some(),
    });
}

/// Canonicalize forwarding definitions before rewriting caller-supplied path suffixes.
pub(crate) fn resolve_result_module_scope(
    scope: &ResultAliasScope,
    lookup: impl Fn(&str) -> Option<ResultAliasScope>,
) -> ResultAliasScope {
    resolve_namespace_query(scope.clone(), &lookup, &mut AHashSet::new(), &mut AHashMap::new())
        .unwrap_or(ResultAliasScope::Foreign)
}

fn resolve_namespace_query(
    mut scope: ResultAliasScope,
    lookup: &impl Fn(&str) -> Option<ResultAliasScope>,
    active: &mut AHashSet<String>,
    targets: &mut AHashMap<String, Option<ResultAliasScope>>,
) -> Option<ResultAliasScope> {
    loop {
        let ResultAliasScope::Crate(module) = &scope else {
            return Some(scope);
        };
        let Some((length, target)) = namespace_alias_prefix(module, lookup) else {
            return Some(scope);
        };
        let prefix = module[..length].to_string();
        let suffix = module[length..].trim_start_matches("::").to_string();
        let target = resolve_namespace_target(&prefix, target, lookup, active, targets)?;
        scope = append_namespace_suffix(target, &suffix);
    }
}

fn resolve_namespace_target(
    prefix: &str,
    target: ResultAliasScope,
    lookup: &impl Fn(&str) -> Option<ResultAliasScope>,
    active: &mut AHashSet<String>,
    targets: &mut AHashMap<String, Option<ResultAliasScope>>,
) -> Option<ResultAliasScope> {
    if let Some(cached) = targets.get(prefix) {
        return cached.clone();
    }
    if !active.insert(prefix.to_string()) {
        return None;
    }
    // Only target definitions keep an edge active; caller suffixes may reuse the same forwarding.
    let resolved = resolve_namespace_query(target, lookup, active, targets);
    active.remove(prefix);
    targets.insert(prefix.to_string(), resolved.clone());
    resolved
}

fn append_namespace_suffix(target: ResultAliasScope, suffix: &str) -> ResultAliasScope {
    match target {
        ResultAliasScope::Foreign => ResultAliasScope::Foreign,
        ResultAliasScope::Crate(target) if target.is_empty() => ResultAliasScope::Crate(suffix.to_string()),
        ResultAliasScope::Crate(target) if suffix.is_empty() => ResultAliasScope::Crate(target),
        ResultAliasScope::Crate(target) => ResultAliasScope::Crate(format!("{target}::{suffix}")),
    }
}

fn namespace_alias_prefix(
    module: &str,
    lookup: &impl Fn(&str) -> Option<ResultAliasScope>,
) -> Option<(usize, ResultAliasScope)> {
    let mut prefix = module;
    loop {
        if let Some(target) = lookup(prefix) {
            return Some((prefix.len(), target));
        }
        prefix = prefix.rsplit_once("::")?.0;
    }
}

/// Turn the path segments preceding an imported `Result` into the module that declares it.
///
/// `crate`/`self`/`super` prefixes and uniform paths into a locally declared module resolve
/// against `module_path`; anything else names another crate, whose `Result` carries its own error
/// type and must not borrow this crate's.
fn scope_for_prefix(prefix: &[String], module_path: &str, local_namespaces: &AHashSet<String>) -> ResultAliasScope {
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
        Some(first) if local_namespaces.contains(first) => base = split_module_path(module_path),
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
