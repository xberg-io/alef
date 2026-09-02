mod defaults;
pub(crate) mod disambiguation;
mod functions;
pub(crate) mod helpers;
mod paths;
mod postprocess;
mod reexports;
pub(crate) mod service;
mod types;

use std::path::{Path, PathBuf};

use crate::core::ir::{ApiSurface, DefaultValue, MethodDef, TypeDef, UnsupportedPublicItem};
use ahash::AHashMap;
use anyhow::{Context, Result};

use crate::extract::type_resolver;

use self::functions::{
    collect_complete_serde_type_names, detect_receiver, extract_function, extract_impl_block, extract_params,
    resolve_return_type,
};
use self::helpers::{
    ResultModuleContextGuard, build_rust_path, collect_reexport_map, extract_binding_exclusion_reason,
    extract_doc_comments, extract_version_annotation, is_pub, is_test_gated, is_thiserror_enum,
    resolve_result_alias_scope,
};
use self::paths::{apply_parent_reexport_shortening, derive_module_path};
use self::postprocess::{
    resolve_enum_field_defaults, resolve_newtypes, resolve_public_default_functions, resolve_trait_sources,
    warn_on_default_disagreements,
};
use self::reexports::{extract_module, resolve_use_tree};
use self::types::{extract_enum, extract_error_enum, extract_struct};

/// A struct's field name → the serde reader's default for that field, keyed by the struct's
/// `rust_path` at the moment it was extracted. Threaded from `extract_struct`
/// (`extract::extractor::types`) through `extract_items`/`extract_module` and accumulated across
/// every source file, so a manual `impl Default` — parsed later, possibly from a different
/// source file — can still be compared against the serde reader's value once the whole crate has
/// been extracted. See `postprocess::warn_on_default_disagreements`.
///
/// Keyed by `rust_path` as extracted, not re-resolved after reexport shortening or
/// disambiguation rename the path: both of those run only after the struct's own
/// `extract_items` call returns, so a manual `impl Default` in the *same* module tree is always
/// compared before its type's path could drift. A manual `impl Default` reached only through a
/// later-renamed cross-module path silently loses the comparison instead of risking a false
/// positive against the wrong key. ~keep
pub(crate) type SerdeDefaultsByType = AHashMap<String, AHashMap<String, DefaultValue>>;

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
    // Spans the whole `sources` loop below (not reset per file, unlike `result_wrapping_aliases`):
    // a struct and the manual `impl Default` that overwrites its fields' defaults may live in
    // different source files. ~keep
    let mut pending_serde_defaults: SerdeDefaultsByType = AHashMap::new();

    type_resolver::reset_result_error_hints();

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
            &mut pending_serde_defaults,
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
    // Every source is scanned, not just the first: a crate's `#[cfg(feature = "x")] pub mod x;`
    // declarations live in whichever file owns them, and `sources` is an author-ordered list
    // with no guarantee that lib.rs comes first. Scanning only `sources.first()` silently left
    // every item under a gated module ungated, so backends that exclude on `cfg` emitted calls
    // into modules absent from their feature set. ~keep
    for source in sources {
        if let Ok(content) = std::fs::read_to_string(source)
            && let Ok(file) = syn::parse_file(&content)
        {
            apply_reexport_cfg_attributes(&mut surface, &file.items);
        }
    }

    // NOTE: Same-named function entries with disjoint cfg gates (e.g. a `pub use real::fn` under
    // `#[cfg(feature = "X")]` plus a stub `pub fn fn(...) -> Err(...)` under
    // `#[cfg(all(feature = "X-presets", not(feature = "X")))]`) are intentionally NOT collapsed

    resolve_trait_sources(&mut surface);

    resolve_public_default_functions(&mut surface);

    resolve_newtypes(&mut surface);

    resolve_enum_field_defaults(&mut surface);

    // Deferred to here rather than run inline while each source file was still being walked: it
    // needs every enum in the crate already extracted to tell a genuine agreement (an enum field
    // set to its own `#[default]` variant) apart from a real disagreement. See
    // `postprocess::warn_on_default_disagreements`. ~keep
    warn_on_default_disagreements(&surface, &pending_serde_defaults);

    disambiguation::disambiguate_type_names(&mut surface);

    // A type returned only wrapped (`Option<T>`, `Vec<T>`, or as a `Map` value) is just as much
    // an output type as one returned bare — `TypeRef::references_named` already walks those
    // wrappers, so reuse it instead of only matching the bare `TypeRef::Named` case. Backends
    // (pyo3's public-dataclass-vs-native-pyclass split in particular) decide a type's DTO shape
    // from this flag alone; missing a wrapped return silently reclassified an output-only type
    // as an input type, so the wrapper's declared annotation and the value it actually produced
    // named different classes. ~keep
    for typ in &mut surface.types {
        if surface
            .functions
            .iter()
            .any(|f| f.return_type.references_named(&typ.name))
        {
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
    pending_serde_defaults: &mut SerdeDefaultsByType,
) -> Result<()> {
    let reexport_map = collect_reexport_map(items);

    // Computed once per module, ahead of the item loop below, so struct extraction can fold a
    // `#[serde(default = "path")]` field's function-call default to a literal at the point the
    // struct's `TypeDef` is built (see the `syn::Item::Struct` arm below). The impl-block loop
    // further down, which reads `impl Default` bodies, reuses the same module-scoped indexes. ~keep
    let literal_consts = defaults::collect_literal_consts(items);
    // Indexed once per module so a `fn default()` that delegates to `Self::new(..)` can be
    // followed to the constructor's own struct literal instead of collapsing to a type-zero, and
    // so a `#[serde(default = "Owner::method")]` path can be resolved to the same function. ~keep
    let constructors = defaults::collect_constructors(items);
    let free_functions = defaults::collect_free_functions(items);

    let mut declares_result_alias = false;
    for item in items {
        if let syn::Item::Type(item_type) = item
            && is_pub(&item_type.vis)
        {
            let name = item_type.ident.to_string();
            // A crate-local `Result` alias almost always carries its own generic
            // parameter (`pub type Result<T> = std::result::Result<T, MyError>;`), so
            // the hint lookup must not be gated on the alias itself being non-generic. ~keep
            if name == "Result"
                && let Some(error_type) =
                    type_resolver::extract_result_error_type_from_alias(&item_type.ty, &item_type.generics)
            {
                // Keyed by the declaring module: a crate may declare several private `Result`
                // aliases, and a name-keyed hint would let whichever module happens to be walked
                // last decide the error type for the whole crate. ~keep
                type_resolver::record_result_error_hint(module_path, error_type);
                declares_result_alias = true;
            }
            if !item_type.generics.params.is_empty() {
                let rhs = quote::quote!(#item_type).to_string();
                if rhs.contains("Result <") || rhs.contains("Result<") {
                    result_wrapping_aliases.insert(name);
                }
            }
        }
    }

    // A module resolves `Result` through its own declaration first, then through whatever its
    // `use` statements import, and only then through the crate's canonical alias. The guard
    // restores the enclosing module's scope when this module's items are done. ~keep
    let alias_scope = if declares_result_alias {
        Some(type_resolver::ResultAliasScope::Crate(module_path.to_string()))
    } else {
        resolve_result_alias_scope(items, module_path)
    };
    let _result_alias_scope = type_resolver::ResultAliasScopeGuard::enter(alias_scope);
    // A return type that qualifies its `Result` (`crate::Result<T>`) resolves against the module
    // it is written in, which the type alone does not carry. ~keep
    let _result_module_context = ResultModuleContextGuard::enter(items, module_path);

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
                if let Some((mut td, serde_defaults)) = extract_struct(item_struct, crate_name, module_path) {
                    defaults::fold_constant_default_functions(
                        &mut td.fields,
                        &free_functions,
                        &constructors,
                        &literal_consts,
                    );
                    if !serde_defaults.is_empty() {
                        pending_serde_defaults.insert(td.rust_path.clone(), serde_defaults);
                    }
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
                    serde_container_default: false,
                    serde_container_conversion: Default::default(),
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

                            if !is_async
                                && let Some((inner, future_error_type)) =
                                    functions::unwrap_future_return(&method.sig.output, result_wrapping_aliases)
                                {
                                    is_async = true;
                                    return_type = inner;
                                    if future_error_type.is_some() {
                                        error_type = future_error_type;
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
                                cfg: None,
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
                    serde_container_default: false,
                    serde_container_conversion: Default::default(),
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
                        pending_serde_defaults,
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
    let binding_excluded_type_names: ahash::AHashSet<String> = items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Struct(item) if extract_binding_exclusion_reason(&item.attrs).is_some() => {
                Some(item.ident.to_string())
            }
            syn::Item::Enum(item) if extract_binding_exclusion_reason(&item.attrs).is_some() => {
                Some(item.ident.to_string())
            }
            _ => None,
        })
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
                &binding_excluded_type_names,
                result_wrapping_aliases,
                &literal_consts,
                &constructors,
                &free_functions,
            );
        }
    }

    // Merge derive and manual impl evidence so asymmetric serde implementations are recognized.
    let complete_serde_names = collect_complete_serde_type_names(items);
    if !complete_serde_names.is_empty() {
        for typ in &mut surface.types {
            if !typ.has_serde && complete_serde_names.contains(&typ.name) {
                typ.has_serde = true;
            }
        }
        for enum_def in &mut surface.enums {
            if !enum_def.has_serde && complete_serde_names.contains(&enum_def.name) {
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
/// A `#[cfg(X)] pub use mod::fn` re-export contributes the cfg gate, never the
/// decision about whether the symbol is part of the binding surface:
///
/// - A declared `#[cfg_attr(alef, alef(skip))]` or `#[doc(hidden)]` on the source
///   item wins. Re-exporting a skipped symbol must not resurrect it — a re-export
///   is how ordinary Rust crates publish their module tree, not an opt-in to
///   binding generation, and resurrecting the item made an explicitly skipped
///   function abort the run with `lossy_sanitized_surface`. ~keep
/// - If no same-named function exists at the re-export cfg (typically because
///   the source is generic and was dropped at extract time), and a concrete
///   same-named entry exists under a disjoint cfg (the `not(X)` stub pattern),
///   clone that concrete entry under the re-export's cfg. The cloned entry
///   compiles to a call against the crate-root path, which the linker resolves
///   to whichever cfg-enabled implementation is active at build time. Only a
///   non-excluded entry is ever cloned, so a skipped source stays skipped. ~keep
fn apply_cfg_to_item(surface: &mut ApiSurface, name: &str, cfg: &str) {
    for typ in &mut surface.types {
        if typ.name == name && typ.cfg.is_none() {
            typ.cfg = Some(cfg.to_string());
        }
    }
    for func in &mut surface.functions {
        if func.name == name && func.cfg.is_none() {
            func.cfg = Some(cfg.to_string());
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
