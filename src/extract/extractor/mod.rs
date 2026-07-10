mod defaults;
mod disambiguation;
mod functions;
mod helpers;
mod paths;
mod postprocess;
mod reexports;
pub(crate) mod service;
mod types;

use std::path::{Path, PathBuf};

use crate::core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef, UnsupportedPublicItem};
use ahash::AHashMap;
use anyhow::{Context, Result};

use crate::extract::type_resolver;

use self::functions::{
    collect_manual_serde_type_names, detect_receiver, extract_function, extract_impl_block, extract_params,
    resolve_return_type,
};
use self::helpers::{
    build_rust_path, collect_reexport_map, extract_binding_exclusion_reason, extract_doc_comments,
    extract_version_annotation, is_pub, is_test_gated, is_thiserror_enum,
};
use self::paths::{apply_parent_reexport_shortening, derive_module_path};
use self::postprocess::{resolve_newtypes, resolve_trait_sources};
use self::reexports::{extract_module, resolve_use_tree};
use self::types::{extract_enum, extract_error_enum, extract_struct};

/// Extract the public API surface from Rust source files.
///
/// `sources` should be the root source files (e.g., `lib.rs`) of the crate.
/// Submodules referenced via `mod` declarations are resolved and extracted recursively.
/// `workspace_root` enables resolution of `pub use` re-exports from workspace sibling crates.
pub fn extract(
    sources: &[&Path],
    crate_name: &str,
    version: &str,
    workspace_root: Option<&Path>,
) -> Result<ApiSurface> {
    let mut surface = ApiSurface {
        crate_name: crate_name.to_string(),
        version: version.to_string(),
        ..ApiSurface::default()
    };

    let mut visited = Vec::<PathBuf>::new();

    let crate_src_dir = sources.first().and_then(|s| s.parent()).map(|p| p.to_path_buf());

    for source in sources {
        let canonical = std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());

        if visited.contains(&canonical) {
            continue;
        }
        visited.push(canonical);

        let content = std::fs::read_to_string(source)
            .with_context(|| format!("Failed to read source file: {}", source.display()))?;
        let file =
            syn::parse_file(&content).with_context(|| format!("Failed to parse source file: {}", source.display()))?;

        let module_path = derive_module_path(source, crate_src_dir.as_deref());

        let types_before = surface.types.len();
        let enums_before = surface.enums.len();
        let fns_before = surface.functions.len();

        let mut result_wrapping_aliases = ahash::AHashSet::new();
        extract_items(
            &file.items,
            source,
            crate_name,
            &module_path,
            &mut surface,
            workspace_root,
            &mut visited,
            &mut result_wrapping_aliases,
        )?;

        if !module_path.is_empty() {
            apply_parent_reexport_shortening(
                source,
                crate_name,
                &module_path,
                &mut surface,
                types_before,
                enums_before,
                fns_before,
            );
        }
    }

    // For intra-crate re-exports like `#[cfg(feature = "api")] pub use core::ServerConfig`,
    if let Some(first_source) = sources.first() {
        if let Ok(content) = std::fs::read_to_string(first_source) {
            if let Ok(file) = syn::parse_file(&content) {
                apply_reexport_cfg_attributes(&mut surface, &file.items);
            }
        }
    }

    // NOTE: Same-named function entries with disjoint cfg gates (e.g. a `pub use real::fn` under
    // `#[cfg(feature = "X")]` plus a stub `pub fn fn(...) -> Err(...)` under
    // `#[cfg(all(feature = "X-presets", not(feature = "X")))]`) are intentionally NOT collapsed

    resolve_trait_sources(&mut surface);

    resolve_newtypes(&mut surface);

    disambiguation::disambiguate_type_names(&mut surface);

    let return_type_names: ahash::AHashSet<String> = surface
        .functions
        .iter()
        .filter_map(|f| match &f.return_type {
            TypeRef::Named(name) => Some(name.clone()),
            _ => None,
        })
        .collect();
    for typ in &mut surface.types {
        if return_type_names.contains(&typ.name) {
            typ.is_return_type = true;
        }
    }

    Ok(surface)
}

fn has_non_lifetime_generics(generics: &syn::Generics) -> bool {
    generics
        .params
        .iter()
        .any(|param| !matches!(param, syn::GenericParam::Lifetime(_)))
}

fn unsupported_public_item(
    item_kind: &str,
    crate_name: &str,
    module_path: &str,
    name: &str,
    reason: &str,
) -> UnsupportedPublicItem {
    UnsupportedPublicItem {
        item_kind: item_kind.to_string(),
        item_path: build_rust_path(crate_name, module_path, name),
        reason: reason.to_string(),
        suggested_fix:
            "exclude the item, configure an opaque/bridge policy, or provide explicit monomorphization metadata"
                .to_string(),
    }
}

/// Extract items from a parsed syn file or module.
#[allow(clippy::too_many_arguments)]
fn extract_items(
    items: &[syn::Item],
    source_path: &Path,
    crate_name: &str,
    module_path: &str,
    surface: &mut ApiSurface,
    workspace_root: Option<&Path>,
    visited: &mut Vec<PathBuf>,
    result_wrapping_aliases: &mut ahash::AHashSet<String>,
) -> Result<()> {
    let reexport_map = collect_reexport_map(items);

    let mut result_error_hints = ahash::AHashMap::new();
    for item in items {
        if let syn::Item::Type(item_type) = item {
            if is_pub(&item_type.vis) {
                let name = item_type.ident.to_string();
                if item_type.generics.params.is_empty() {
                    if name == "Result" {
                        if let Some(error_type) = type_resolver::extract_result_error_type_from_alias(&item_type.ty) {
                            result_error_hints.insert(name.clone(), error_type);
                        }
                    }
                } else {
                    let rhs = quote::quote!(#item_type).to_string();
                    if rhs.contains("Result <") || rhs.contains("Result<") {
                        result_wrapping_aliases.insert(name);
                    }
                }
            }
        }
    }
    type_resolver::set_result_error_hints(result_error_hints);

    for item in items {
        // `#[cfg(test)]` items do not exist in normal builds; skip them so the
        if item_attrs(item).is_some_and(is_test_gated) {
            continue;
        }
        match item {
            syn::Item::Struct(item_struct) if is_pub(&item_struct.vis) => {
                if has_non_lifetime_generics(&item_struct.generics) {
                    // Generic items annotated with `#[alef::skip]` (or `#[doc(hidden)]`) are
                    if extract_binding_exclusion_reason(&item_struct.attrs).is_none() {
                        surface.unsupported_public_items.push(unsupported_public_item(
                            "struct",
                            crate_name,
                            module_path,
                            &item_struct.ident.to_string(),
                            "public generic structs cannot be represented without explicit monomorphization metadata",
                        ));
                    }
                    continue;
                }
                if let Some(td) = extract_struct(item_struct, crate_name, module_path) {
                    surface.types.push(td);
                }
            }
            syn::Item::Enum(item_enum) if is_pub(&item_enum.vis) => {
                if has_non_lifetime_generics(&item_enum.generics) {
                    if extract_binding_exclusion_reason(&item_enum.attrs).is_none() {
                        surface.unsupported_public_items.push(unsupported_public_item(
                            "enum",
                            crate_name,
                            module_path,
                            &item_enum.ident.to_string(),
                            "public generic enums cannot be represented without explicit monomorphization metadata",
                        ));
                    }
                    continue;
                }
                if is_thiserror_enum(&item_enum.attrs) {
                    if let Some(ed) = extract_error_enum(item_enum, crate_name, module_path) {
                        surface.errors.push(ed);
                    }
                } else if let Some(ed) = extract_enum(item_enum, crate_name, module_path) {
                    surface.enums.push(ed);
                }
            }
            syn::Item::Fn(item_fn) if is_pub(&item_fn.vis) && !item_fn.sig.ident.to_string().starts_with('_') => {
                if has_non_lifetime_generics(&item_fn.sig.generics) {
                    if extract_binding_exclusion_reason(&item_fn.attrs).is_none() {
                        surface.unsupported_public_items.push(unsupported_public_item(
                            "function",
                            crate_name,
                            module_path,
                            &item_fn.sig.ident.to_string(),
                            "public generic functions cannot be represented without explicit monomorphization metadata",
                        ));
                    }
                    continue;
                }
                if let Some(fd) = extract_function(item_fn, crate_name, module_path) {
                    surface.functions.push(fd);
                }
            }
            syn::Item::Type(item_type) if is_pub(&item_type.vis) && has_non_lifetime_generics(&item_type.generics) => {
                let alias_name = item_type.ident.to_string();
                let is_result_wrapping = alias_name == "Result" || result_wrapping_aliases.contains(&alias_name);
                if !is_result_wrapping && extract_binding_exclusion_reason(&item_type.attrs).is_none() {
                    surface.unsupported_public_items.push(unsupported_public_item(
                        "type_alias",
                        crate_name,
                        module_path,
                        &alias_name,
                        "public generic type aliases cannot be represented without explicit monomorphization metadata",
                    ));
                }
            }
            syn::Item::Type(item_type) if is_pub(&item_type.vis) && item_type.generics.params.is_empty() => {
                let name = item_type.ident.to_string();
                let _ty = type_resolver::resolve_type(&item_type.ty);
                let rust_path = build_rust_path(crate_name, module_path, &name);
                let doc = extract_doc_comments(&item_type.attrs);
                let binding_exclusion_reason = extract_binding_exclusion_reason(&item_type.attrs);
                let binding_excluded = binding_exclusion_reason.is_some();
                surface.types.push(TypeDef {
                    name,
                    rust_path,
                    original_rust_path: String::new(),
                    fields: vec![],
                    methods: vec![],
                    is_opaque: true,
                    is_clone: false,
                    is_copy: false,
                    is_trait: false,
                    has_default: false,
                    has_stripped_cfg_fields: false,
                    is_return_type: false,
                    doc,
                    cfg: None,
                    serde_rename_all: None,
                    has_serde: false,
                    super_traits: vec![],
                    binding_excluded,
                    binding_exclusion_reason,
                    is_variant_wrapper: false,
                    has_lifetime_params: false,
                    has_private_fields: false,
                    version: extract_version_annotation(&item_type.attrs),
                });
            }
            syn::Item::Trait(item_trait)
                if is_pub(&item_trait.vis) && has_non_lifetime_generics(&item_trait.generics) =>
            {
                if extract_binding_exclusion_reason(&item_trait.attrs).is_none() {
                    surface.unsupported_public_items.push(unsupported_public_item(
                        "trait",
                        crate_name,
                        module_path,
                        &item_trait.ident.to_string(),
                        "public generic traits cannot be represented without explicit monomorphization metadata",
                    ));
                }
            }
            syn::Item::Trait(item_trait) if is_pub(&item_trait.vis) && item_trait.generics.params.is_empty() => {
                let name = item_trait.ident.to_string();
                let rust_path = build_rust_path(crate_name, module_path, &name);
                let doc = extract_doc_comments(&item_trait.attrs);
                let trait_binding_exclusion_reason = extract_binding_exclusion_reason(&item_trait.attrs);
                let trait_binding_excluded = trait_binding_exclusion_reason.is_some();

                let methods: Vec<MethodDef> = item_trait
                    .items
                    .iter()
                    .filter_map(|item| {
                        if let syn::TraitItem::Fn(method) = item {
                            let method_name = method.sig.ident.to_string();
                            let method_doc = extract_doc_comments(&method.attrs);
                            let method_binding_exclusion_reason = extract_binding_exclusion_reason(&method.attrs);
                            let method_binding_excluded = method_binding_exclusion_reason.is_some();
                            let mut is_async = method.sig.asyncness.is_some();
                            let (mut return_type, mut error_type, returns_ref) =
                                resolve_return_type(&method.sig.output);

                            if !is_async {
                                if let Some((inner, future_error_type)) =
                                    functions::unwrap_future_return(&method.sig.output, result_wrapping_aliases)
                                {
                                    is_async = true;
                                    return_type = inner;
                                    if future_error_type.is_some() {
                                        error_type = future_error_type;
                                    }
                                }
                            }

                            if !method.sig.generics.params.is_empty() {
                                if method_binding_exclusion_reason.is_none() {
                                    surface.unsupported_public_items.push(UnsupportedPublicItem {
                                        item_kind: "method".to_string(),
                                        item_path: format!("{rust_path}.{method_name}"),
                                        reason: "public generic trait methods cannot be represented without explicit monomorphization metadata".to_string(),
                                        suggested_fix: "exclude the method, configure an opaque/bridge policy, or provide explicit monomorphization metadata".to_string(),
                                    });
                                }
                                return None;
                            }

                            let (receiver, is_static) = detect_receiver(&method.sig.inputs);
                            let params = extract_params(&method.sig.inputs);

                            Some(MethodDef {
                                name: method_name,
                                params,
                                return_type,
                                is_async,
                                is_static,
                                error_type,
                                doc: method_doc,
                                receiver,
                                sanitized: false,
                                trait_source: None,
                                returns_ref,
                                returns_cow: false,
                                return_newtype_wrapper: None,
                                has_default_impl: method.default.is_some(),
                                binding_excluded: method_binding_excluded,
                                binding_exclusion_reason: method_binding_exclusion_reason,
                                version: extract_version_annotation(&method.attrs),
                            })
                        } else {
                            None
                        }
                    })
                    .collect();

                let super_traits: Vec<String> = item_trait
                    .supertraits
                    .iter()
                    .filter_map(|bound| {
                        if let syn::TypeParamBound::Trait(trait_bound) = bound {
                            let path = &trait_bound.path;
                            let name = path.segments.last()?.ident.to_string();
                            if name == "Send" || name == "Sync" || name == "Sized" {
                                None
                            } else {
                                Some(name)
                            }
                        } else {
                            None
                        }
                    })
                    .collect();

                surface.types.push(TypeDef {
                    name,
                    rust_path,
                    original_rust_path: String::new(),
                    fields: vec![],
                    methods,
                    is_opaque: true,
                    is_clone: false,
                    is_copy: false,
                    is_trait: true,
                    has_default: false,
                    has_stripped_cfg_fields: false,
                    is_return_type: false,
                    doc,
                    cfg: None,
                    serde_rename_all: None,
                    has_serde: false,
                    super_traits,
                    binding_excluded: trait_binding_excluded,
                    binding_exclusion_reason: trait_binding_exclusion_reason,
                    is_variant_wrapper: false,
                    has_lifetime_params: false,
                    has_private_fields: false,
                    version: extract_version_annotation(&item_trait.attrs),
                });
            }
            syn::Item::Mod(item_mod) => {
                let mod_name = item_mod.ident.to_string();
                let is_reexported = reexport_map.contains_key(&mod_name);
                if is_pub(&item_mod.vis) || is_reexported {
                    extract_module(
                        item_mod,
                        source_path,
                        crate_name,
                        module_path,
                        &reexport_map,
                        surface,
                        workspace_root,
                        visited,
                    )?;
                }
            }
            syn::Item::Use(item_use) if is_pub(&item_use.vis) => {
                resolve_use_tree(
                    &item_use.tree,
                    crate_name,
                    surface,
                    workspace_root,
                    visited,
                    &item_use.attrs,
                )?;
            }
            _ => {}
        }
    }

    let type_index: AHashMap<String, usize> = surface
        .types
        .iter()
        .enumerate()
        .map(|(idx, typ)| (typ.name.clone(), idx))
        .collect();

    for item in items {
        if let syn::Item::Impl(item_impl) = item {
            // A whole `#[cfg(test)]` impl block (e.g. test-only constructors) is
            if is_test_gated(&item_impl.attrs) {
                continue;
            }
            extract_impl_block(
                item_impl,
                crate_name,
                module_path,
                surface,
                &type_index,
                result_wrapping_aliases,
            );
        }
    }

    // The struct/enum extractor only sets has_serde=true when #[derive(Serialize, Deserialize)]
    let manual_serde_names = collect_manual_serde_type_names(items);
    if !manual_serde_names.is_empty() {
        for typ in &mut surface.types {
            if !typ.has_serde && manual_serde_names.contains(&typ.name) {
                typ.has_serde = true;
            }
        }
        for enum_def in &mut surface.enums {
            if !enum_def.has_serde && manual_serde_names.contains(&enum_def.name) {
                enum_def.has_serde = true;
            }
        }
    }

    Ok(())
}

/// Return the outer attributes of an item for the variants that can carry a
/// `#[cfg(test)]` gate and are extracted into the binding surface. Other item
/// kinds (uses, mods, macros, …) are handled by dedicated passes.
fn item_attrs(item: &syn::Item) -> Option<&[syn::Attribute]> {
    match item {
        syn::Item::Struct(i) => Some(&i.attrs),
        syn::Item::Enum(i) => Some(&i.attrs),
        syn::Item::Fn(i) => Some(&i.attrs),
        syn::Item::Type(i) => Some(&i.attrs),
        syn::Item::Trait(i) => Some(&i.attrs),
        syn::Item::Impl(i) => Some(&i.attrs),
        _ => None,
    }
}

/// Apply cfg attributes from pub use and pub mod statements to extracted items.
///
/// For example:
/// - `#[cfg(feature = "api")] pub use core::ServerConfig` marks ServerConfig with cfg
/// - `#[cfg(feature = "api")] pub mod api { ... }` marks all items from api module with cfg
fn apply_reexport_cfg_attributes(surface: &mut ApiSurface, items: &[syn::Item]) {
    for item in items {
        match item {
            syn::Item::Use(item_use) if helpers::is_pub(&item_use.vis) => {
                if let Some(cfg_str) = helpers::extract_cfg_condition(&item_use.attrs) {
                    collect_reexport_names_with_cfg(&item_use.tree, surface, &cfg_str);
                }
            }
            syn::Item::Mod(item_mod) if helpers::is_pub(&item_mod.vis) => {
                if let Some(cfg_str) = helpers::extract_cfg_condition(&item_mod.attrs) {
                    apply_module_cfg(surface, &item_mod.ident.to_string(), &cfg_str);
                }
            }
            _ => {}
        }
    }
}

/// Extract names from a use tree and apply cfg to matching items in the surface.
fn collect_reexport_names_with_cfg(tree: &syn::UseTree, surface: &mut ApiSurface, cfg: &str) {
    match tree {
        syn::UseTree::Path(use_path) => {
            collect_reexport_names_with_cfg(&use_path.tree, surface, cfg);
        }
        syn::UseTree::Name(name) => {
            let item_name = name.ident.to_string();
            apply_cfg_to_item(surface, &item_name, cfg);
        }
        syn::UseTree::Rename(rename) => {
            let item_name = rename.rename.to_string();
            apply_cfg_to_item(surface, &item_name, cfg);
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_reexport_names_with_cfg(item, surface, cfg);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

/// Apply a cfg attribute to an item in the surface by name.
///
/// A `#[cfg(X)] pub use mod::fn` re-export is treated as the canonical public
/// binding surface, even when the underlying source carries `#[alef(skip)]` or
/// `#[doc(hidden)]`:
///
/// - If a same-named function already exists in the surface but is
///   `binding_excluded`, clear the exclusion. The re-export publicly republishes
///   the symbol, so the skip annotation on the private source is overridden.
/// - If no same-named function exists at the re-export cfg (typically because
///   the source is generic and was dropped at extract time), and a concrete
///   same-named entry exists under a disjoint cfg (the `not(X)` stub pattern),
///   clone that concrete entry under the re-export's cfg. The cloned entry
///   compiles to a call against the crate-root path, which the linker resolves
///   to whichever cfg-enabled implementation is active at build time.
fn apply_cfg_to_item(surface: &mut ApiSurface, name: &str, cfg: &str) {
    for typ in &mut surface.types {
        if typ.name == name && typ.cfg.is_none() {
            typ.cfg = Some(cfg.to_string());
        }
    }
    for func in &mut surface.functions {
        if func.name != name {
            continue;
        }
        if func.cfg.is_none() {
            func.cfg = Some(cfg.to_string());
        }
        if func.binding_excluded {
            func.binding_excluded = false;
            func.binding_exclusion_reason = None;
        }
    }
    for en in &mut surface.enums {
        if en.name == name && en.cfg.is_none() {
            en.cfg = Some(cfg.to_string());
        }
    }

    let has_matching_cfg = surface
        .functions
        .iter()
        .any(|f| f.name == name && f.cfg.as_deref() == Some(cfg));
    if !has_matching_cfg {
        let stub_opt = surface
            .functions
            .iter()
            .find(|f| f.name == name && !f.binding_excluded)
            .cloned();
        if let Some(stub) = stub_opt {
            let mut paired = stub;
            paired.cfg = Some(cfg.to_string());
            paired.binding_excluded = false;
            paired.binding_exclusion_reason = None;
            surface.functions.push(paired);
        }
    }
}

/// Apply a cfg attribute to all items from a module.
///
/// For example, if `pub mod api` is gated behind `#[cfg(feature = "api")]`,
/// all items whose rust_path starts with `{crate_name}::api::` should be marked with that cfg.
fn apply_module_cfg(surface: &mut ApiSurface, module_name: &str, cfg: &str) {
    let module_prefix = format!("::{module_name}::");
    let module_prefix_self = format!("::{module_name}");

    for typ in &mut surface.types {
        if typ.cfg.is_none() && (typ.rust_path.contains(&module_prefix) || typ.rust_path.ends_with(&module_prefix_self))
        {
            typ.cfg = Some(cfg.to_string());
        }
    }
    for func in &mut surface.functions {
        if func.cfg.is_none()
            && (func.rust_path.contains(&module_prefix) || func.rust_path.ends_with(&module_prefix_self))
        {
            func.cfg = Some(cfg.to_string());
        }
    }
    for en in &mut surface.enums {
        if en.cfg.is_none() && (en.rust_path.contains(&module_prefix) || en.rust_path.ends_with(&module_prefix_self)) {
            en.cfg = Some(cfg.to_string());
        }
    }
}

#[cfg(test)]
mod tests;
