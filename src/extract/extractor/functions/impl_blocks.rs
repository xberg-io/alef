use crate::core::ir::{ApiSurface, DefaultValue, MethodDef, TypeDef, UnsupportedPublicItem};
use ahash::AHashMap;

use super::super::defaults::{
    ConstructorIndex, FreeFunctionIndex, extract_default_values, fold_constant_default_functions,
};
use super::super::helpers::{build_rust_path, extract_binding_exclusion_reason, extract_cfg_condition, is_test_gated};
use super::extract_method;

fn has_non_lifetime_generics(generics: &syn::Generics) -> bool {
    generics
        .params
        .iter()
        .any(|param| !matches!(param, syn::GenericParam::Lifetime(_)))
}

fn record_unsupported_generic_impl_methods(
    item: &syn::ItemImpl,
    crate_name: &str,
    type_name: &str,
    surface: &mut ApiSurface,
    reason: &str,
    methods_are_public_by_trait: bool,
) {
    for impl_item in &item.items {
        let syn::ImplItem::Fn(method) = impl_item else {
            continue;
        };
        if (!methods_are_public_by_trait && !super::super::helpers::is_pub(&method.vis))
            || extract_binding_exclusion_reason(&method.attrs).is_some()
        {
            continue;
        }
        let method_name = method.sig.ident.to_string();
        if method_name.starts_with('_') {
            continue;
        }
        surface.unsupported_public_items.push(UnsupportedPublicItem {
            item_kind: "method".to_string(),
            item_path: format!("{crate_name}::{type_name}.{method_name}"),
            reason: reason.to_string(),
            suggested_fix:
                "exclude the method, configure an opaque/bridge policy, or provide explicit monomorphization metadata"
                    .to_string(),
        });
    }
}

/// Extract methods from an `impl` block and attach them to the corresponding `TypeDef`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn extract_impl_block(
    item: &syn::ItemImpl,
    crate_name: &str,
    module_path: &str,
    surface: &mut ApiSurface,
    type_index: &AHashMap<String, usize>,
    binding_excluded_type_names: &ahash::AHashSet<String>,
    result_wrapping_aliases: &ahash::AHashSet<String>,
    literal_consts: &AHashMap<String, DefaultValue>,
    constructors: &ConstructorIndex<'_>,
    free_functions: &FreeFunctionIndex<'_>,
) {
    // Honor `#[cfg_attr(alef, alef(skip))]` (or bare `#[alef(skip)]`) on the impl block
    if extract_binding_exclusion_reason(&item.attrs).is_some() {
        return;
    }

    // The block's own gate applies to every method it contains; `#[cfg(test)]` blocks were
    // already dropped by the caller, so anything left here is a real binding-surface gate. ~keep
    let impl_cfg = extract_cfg_condition(&item.attrs);

    if item.trait_.is_some() {
        extract_trait_impl_methods(
            item,
            crate_name,
            surface,
            type_index,
            result_wrapping_aliases,
            literal_consts,
            impl_cfg.as_deref(),
            constructors,
            free_functions,
        );
        return;
    }

    let type_name = match &*item.self_ty {
        syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default(),
        _ => return,
    };

    if binding_excluded_type_names.contains(&type_name)
        || type_index
            .get(&type_name)
            .is_some_and(|&idx| surface.types[idx].binding_excluded)
    {
        return;
    }

    if has_non_lifetime_generics(&item.generics) {
        record_unsupported_generic_impl_methods(
            item,
            crate_name,
            &type_name,
            surface,
            "public methods on generic impl blocks cannot be represented without explicit monomorphization metadata",
            false,
        );
        return;
    }

    let type_is_opaque = item.generics.params.is_empty()
        && (type_index
            .get(&type_name)
            .map(|&idx| surface.types[idx].is_opaque)
            .unwrap_or(false)
            || surface.enums.iter().any(|e| e.name == type_name)
            || surface.errors.iter().any(|e| e.name == type_name)
            || !type_index.contains_key(&type_name));

    let methods: Vec<MethodDef> = item
        .items
        .iter()
        .filter_map(|impl_item| {
            if let syn::ImplItem::Fn(method) = impl_item
                && super::super::helpers::is_pub(&method.vis) {
                    // Skip `#[cfg(test)]` methods (e.g. test-only constructors like
                    if is_test_gated(&method.attrs) {
                        return None;
                    }
                    if !method.sig.generics.params.is_empty() {
                        if extract_binding_exclusion_reason(&method.attrs).is_none() {
                            surface.unsupported_public_items.push(UnsupportedPublicItem {
                                item_kind: "method".to_string(),
                                item_path: format!("{crate_name}::{type_name}.{}", method.sig.ident),
                                reason: "public generic inherent methods cannot be represented without explicit monomorphization metadata".to_string(),
                                suggested_fix: "exclude the method, configure an opaque/bridge policy, or provide explicit monomorphization metadata".to_string(),
                            });
                        }
                        return None;
                    }
                    let method_name = method.sig.ident.to_string();
                    if method_name.starts_with('_') {
                        return None;
                    }
                    if method_name == "new" && !type_is_opaque
                        && let syn::ReturnType::Type(_, ty) = &method.sig.output
                            && matches!(&**ty, syn::Type::Path(p) if p.path.is_ident("Self")) {
                                return None;
                            }
                    return Some(extract_method(
                        method,
                        crate_name,
                        &type_name,
                        None,
                        result_wrapping_aliases,
                        impl_cfg.as_deref(),
                    ));
                }
            None
        })
        .collect();

    if methods.is_empty() {
        return;
    }

    if let Some(&idx) = type_index.get(&type_name) {
        for method in methods {
            // First-wins by name, with no `cfg` merge: when the same method name is provided by
            // two blocks under disjoint gates (`#[cfg(feature = "x")]` / `#[cfg(not(...))]`), the
            // first block's gate is the one that survives onto the retained `MethodDef`. Free
            // functions have `codegen::fn_dedup` for exactly this; methods have no counterpart
            // yet. Merging the gates (OR of the group, mirroring `with_deduped_functions`) is
            // deliberately deferred — do it here and in the trait-impl loop below together. ~keep
            if !surface.types[idx].methods.iter().any(|m| m.name == method.name) {
                surface.types[idx].methods.push(method);
            }
        }
    } else if let Some(error_def) = surface.errors.iter_mut().find(|e| e.name == type_name) {
        const ERROR_METHOD_WHITELIST: &[&str] = &["status_code", "is_transient", "error_type"];
        for method in methods {
            let is_whitelisted = ERROR_METHOD_WHITELIST.contains(&method.name.as_str());
            let already_present = error_def.methods.iter().any(|m| m.name == method.name);
            if is_whitelisted && !already_present {
                error_def.methods.push(method);
            }
        }
    } else if let Some(enum_def) = surface.enums.iter_mut().find(|e| {
        if e.name != type_name {
            return false;
        }
        let crate_prefix = format!("{crate_name}::");
        let rel = e.rust_path.strip_prefix(&*crate_prefix).unwrap_or(e.rust_path.as_str());
        let enum_module_rel = rel.rfind("::").map(|i| &rel[..i]).unwrap_or("");
        if enum_module_rel.is_empty() {
            return true;
        }
        if module_path.is_empty() {
            return false;
        }
        enum_module_rel.starts_with(module_path) || module_path.starts_with(enum_module_rel)
    }) {
        for method in &methods {
            if method.is_static && !enum_def.methods.iter().any(|m| m.name == method.name) {
                enum_def.methods.push(method.clone());
            }
        }
    } else {
        let rust_path = build_rust_path(crate_name, module_path, &type_name);
        surface.types.push(TypeDef {
            name: type_name.clone(),
            rust_path,
            original_rust_path: String::new(),
            fields: vec![],
            methods,
            is_opaque: true,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            doc: String::new(),
            cfg: None,
            serde_rename_all: None,
            has_serde: false,
            serde_container_default: false,
            serde_container_conversion: Default::default(),
            super_traits: vec![],
            binding_excluded: true,
            binding_exclusion_reason: Some(
                "synthetic-opaque-from-impl-block (source visibility unverified)".to_string(),
            ),
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        });
    }
}

/// The unit variant a hand-written `impl Default for SomeEnum` returns, when the body is a bare
/// `Self::Variant` / `SomeEnum::Variant` tail expression.
///
/// `#[derive(Default)]` records its choice on the variant itself (`EnumVariant::is_default`), but a
/// manual impl carries the same fact only in its body, and every consumer of `is_default` — the Go,
/// Rustler, Dart, WASM, Kotlin, Magnus and PHP backends, plus the generated Rust mirror enum's
/// `#[default]` marker — would otherwise fall back to the *first declared* variant or to no default
/// at all. Both are guesses that silently disagree with the Rust core whenever the real default is
/// declared elsewhere in the enum. Reading it here turns that guess into a fact.
///
/// Deliberately narrow: only a bare path to a unit variant is recognised. A tuple/struct variant, a
/// `match`, or any computed body leaves `is_default` unset, so callers keep their existing honest
/// fallback rather than receiving a fabricated variant. ~keep
fn manual_default_unit_variant(item: &syn::ItemImpl) -> Option<String> {
    let default_fn = item.items.iter().find_map(|impl_item| match impl_item {
        syn::ImplItem::Fn(method) if method.sig.ident == "default" => Some(method),
        _ => None,
    })?;

    let tail = match default_fn.block.stmts.last()? {
        syn::Stmt::Expr(expr, _) => expr,
        _ => return None,
    };
    let expr = match tail {
        syn::Expr::Return(ret) => ret.expr.as_deref()?,
        other => other,
    };
    let syn::Expr::Path(path_expr) = expr else {
        return None;
    };

    let segments = &path_expr.path.segments;
    if segments.len() != 2 {
        return None;
    }
    let qualifier = segments.first()?.ident.to_string();
    let variant = segments.last()?;
    if !variant.arguments.is_none() {
        return None;
    }
    let self_type = match &*item.self_ty {
        syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    };
    (qualifier == "Self" || Some(&qualifier) == self_type.as_ref()).then(|| variant.ident.to_string())
}

/// Extract methods from a trait impl and attach them to an existing type in the surface.
#[allow(clippy::too_many_arguments)]
/// Whether a trait impl names a trait declared OUTSIDE the crates being extracted.
///
/// The trait filter here is otherwise a denylist (`STD_TRAITS`), so any trait not on that list
/// contributes its methods to the public binding surface. That is wrong for a framework trait a
/// consumer implements to serve some other tool: `impl utoipa::ToSchema for Config { fn schema() }`
/// exists for OpenAPI generation, and `schema`/`schemas` are not API a binding caller should ever
/// see — they surface as lossy sanitized methods and abort generation.
///
/// Only a fully-qualified path can be judged reliably. A root of `crate`/`self`/`super`, the crate
/// being extracted, or any crate already contributing a type to this surface is local. Anything
/// else is a foreign crate's trait. A single-segment path (`impl ToSchema for X` after a `use`)
/// is deliberately NOT treated as external: resolving it means asking whether a local trait of
/// that name exists, and during a per-file walk a trait declared in a not-yet-visited module is
/// legitimately absent, so that check would drop real methods depending on `mod` order. ~keep
fn trait_impl_is_external(path: &syn::Path, crate_name: &str, surface: &ApiSurface) -> bool {
    let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    if segments.len() < 2 {
        return false;
    }
    let root = segments[0].replace('-', "_");
    if matches!(root.as_str(), "crate" | "self" | "super") {
        return false;
    }
    if root == crate_name.replace('-', "_") {
        return false;
    }
    let local_root = |rust_path: &str| {
        rust_path
            .split("::")
            .next()
            .is_some_and(|r| r.replace('-', "_") == root)
    };
    !surface.types.iter().any(|t| local_root(&t.rust_path))
        && !surface.enums.iter().any(|e| local_root(&e.rust_path))
        && !surface.functions.iter().any(|f| local_root(&f.rust_path))
}

#[expect(
    clippy::too_many_arguments,
    reason = "one extraction context threaded verbatim from the caller; bundling it would \
              only move the same fields behind a struct used at a single call site"
)]
fn extract_trait_impl_methods(
    item: &syn::ItemImpl,
    crate_name: &str,
    surface: &mut ApiSurface,
    type_index: &AHashMap<String, usize>,
    result_wrapping_aliases: &ahash::AHashSet<String>,
    literal_consts: &AHashMap<String, DefaultValue>,
    impl_cfg: Option<&str>,
    constructors: &ConstructorIndex<'_>,
    free_functions: &FreeFunctionIndex<'_>,
) {
    let type_name = match &*item.self_ty {
        syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    };

    let Some(type_name) = type_name else { return };

    let Some(&idx) = type_index.get(&type_name) else {
        if let Some((path, _)) = &item.trait_
            && path.segments.last().is_some_and(|s| s.ident == "Default")
            && let Some(enum_def) = surface.enums.iter_mut().find(|e| e.name == type_name)
        {
            enum_def.has_default = true;
            if let Some(variant_name) = manual_default_unit_variant(item)
                && let Some(variant) = enum_def
                    .variants
                    .iter_mut()
                    .find(|v| v.name == variant_name && v.fields.is_empty() && !v.originally_had_data_fields)
            {
                variant.is_default = true;
            }
        }
        return;
    };

    if has_non_lifetime_generics(&item.generics) {
        record_unsupported_generic_impl_methods(
            item,
            crate_name,
            &type_name,
            surface,
            "public trait implementation methods on generic impl blocks cannot be represented without explicit monomorphization metadata",
            true,
        );
        return;
    }

    const STD_TRAITS: &[&str] = &[
        "Default",
        "Clone",
        "Copy",
        "Debug",
        "Display",
        "Drop",
        "PartialEq",
        "Eq",
        "PartialOrd",
        "Ord",
        "Hash",
        "From",
        "Into",
        "TryFrom",
        "TryInto",
        "Iterator",
        "IntoIterator",
        "Send",
        "Sync",
        "Sized",
        "Unpin",
        "Serialize",
        "Deserialize",
    ];
    let trait_source = item.trait_.as_ref().and_then(|(path, _)| {
        let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        let trait_name = segments.last().map(|s| s.as_str()).unwrap_or("");
        if STD_TRAITS.contains(&trait_name) {
            return None;
        }
        if segments.len() == 1 {
            let trait_name = &segments[0];
            surface
                .types
                .iter()
                .find(|t| t.is_trait && t.name == *trait_name)
                .map(|t| t.rust_path.replace('-', "_"))
        } else {
            Some(segments.join("::").replace('-', "_"))
        }
    });

    let trait_is_external = item
        .trait_
        .as_ref()
        .is_some_and(|(path, _)| trait_impl_is_external(path, crate_name, surface));

    let type_def = &mut surface.types[idx];

    let is_default_trait_impl = item
        .trait_
        .as_ref()
        .is_some_and(|(path, _)| path.segments.last().is_some_and(|segment| segment.ident == "Default"));
    if is_default_trait_impl {
        // NOTE: this also sets `has_default` for a *manual* `impl Default`, so the flag does not
        // distinguish a derived (type-zero) default from a hand-written one. Telling those apart
        // is `DefaultValue::Unresolved`'s job, not this flag's. ~keep
        let self_type = type_def.name.clone();
        type_def.has_default = true;
        // `warn_on_default_disagreement` is deliberately not called here: at this point in the
        // per-file walk, an enum declared in a source file this crate has not yet reached (or
        // even the same file, later) is still absent from `surface.enums`, so `agrees_via_enum_
        // default` could not prove a genuine agreement and would warn a false positive purely
        // from `mod` declaration order. `postprocess::warn_on_default_disagreements` runs the
        // same check once, after every source file has been extracted. ~keep
        let binding_excluded = type_def.binding_excluded;
        extract_default_values(
            item,
            &self_type,
            &mut type_def.fields,
            literal_consts,
            constructors,
            binding_excluded,
        );
        // `extract_default_values` reads the `impl Default` struct literal with the same
        // constant-folder `fold_constant_default_functions` uses for `#[serde(default =
        // "path")]` fields, but its own `Expr::Call` handling stops at recording a bare
        // `DefaultValue::FunctionCall` for a zero-arg call it does not itself look inside (that
        // arm exists to name a call for every other caller of `expr_to_default_value`, most of
        // whom have no function index to resolve it against). A field whose declared default
        // and whose `impl Default` initializer are the very same free-function call — e.g.
        // `#[serde(default = "default_scheme_allowlist")]` next to `Self { scheme_allowlist:
        // default_scheme_allowlist(), .. }` — would otherwise regress from the folded literal
        // the serde-default pass already proved to a fresh, unfolded `FunctionCall` here,
        // purely because `impl Default` extraction unconditionally overwrites every field's
        // `typed_default`. Re-running the same idempotent fold immediately after closes that
        // gap: a field already folded to a concrete value is untouched (the fold only matches
        // `FunctionCall`), and a field this pass just reset to `FunctionCall` gets exactly the
        // same "read the named function's body if it is one constant-foldable statement"
        // treatment the serde-default path already gets. ~keep
        fold_constant_default_functions(&mut type_def.fields, free_functions, constructors, literal_consts);
    }

    let is_conversion_trait = item.trait_.as_ref().is_some_and(|(path, _)| {
        path.segments
            .last()
            .is_some_and(|s| matches!(s.ident.to_string().as_str(), "From" | "Into" | "TryFrom" | "TryInto"))
    });
    if is_conversion_trait {
        return;
    }

    let is_std_trait_impl = item.trait_.as_ref().is_some_and(|(path, _)| {
        path.segments
            .last()
            .is_some_and(|s| STD_TRAITS.contains(&s.ident.to_string().as_str()))
    });
    if is_std_trait_impl && !is_default_trait_impl {
        return;
    }

    if trait_is_external {
        return;
    }

    for impl_item in &item.items {
        if let syn::ImplItem::Fn(method) = impl_item {
            if !method.sig.generics.params.is_empty() {
                if extract_binding_exclusion_reason(&method.attrs).is_none() {
                    surface.unsupported_public_items.push(UnsupportedPublicItem {
                        item_kind: "method".to_string(),
                        item_path: format!("{crate_name}::{type_name}.{}", method.sig.ident),
                        reason: "public generic trait implementation methods cannot be represented without explicit monomorphization metadata".to_string(),
                        suggested_fix: "exclude the method, configure an opaque/bridge policy, or provide explicit monomorphization metadata".to_string(),
                    });
                }
                continue;
            }
            let method_def = extract_method(
                method,
                crate_name,
                &type_name,
                trait_source.clone(),
                result_wrapping_aliases,
                impl_cfg,
            );
            // First-wins by name, no `cfg` merge — see the note in `extract_impl_block`. ~keep
            if !type_def.methods.iter().any(|m| m.name == method_def.name) {
                type_def.methods.push(method_def);
            }
        }
    }
}
