use crate::core::ir::{ApiSurface, MethodDef, TypeDef, UnsupportedPublicItem};
use ahash::AHashMap;

use super::super::defaults::extract_default_values;
use super::super::helpers::{build_rust_path, extract_binding_exclusion_reason, is_test_gated};
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
pub(crate) fn extract_impl_block(
    item: &syn::ItemImpl,
    crate_name: &str,
    module_path: &str,
    surface: &mut ApiSurface,
    type_index: &AHashMap<String, usize>,
    result_wrapping_aliases: &ahash::AHashSet<String>,
) {
    // Honor `#[cfg_attr(alef, alef(skip))]` (or bare `#[alef(skip)]`) on the impl block
    if extract_binding_exclusion_reason(&item.attrs).is_some() {
        return;
    }

    if item.trait_.is_some() {
        extract_trait_impl_methods(item, crate_name, surface, type_index, result_wrapping_aliases);
        return;
    }

    let type_name = match &*item.self_ty {
        syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default(),
        _ => return,
    };

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
            if let syn::ImplItem::Fn(method) = impl_item {
                if super::super::helpers::is_pub(&method.vis) {
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
                    if method_name == "new" && !type_is_opaque {
                        if let syn::ReturnType::Type(_, ty) = &method.sig.output {
                            if matches!(&**ty, syn::Type::Path(p) if p.path.is_ident("Self")) {
                                return None;
                            }
                        }
                    }
                    return Some(extract_method(
                        method,
                        crate_name,
                        &type_name,
                        None,
                        result_wrapping_aliases,
                    ));
                }
            }
            None
        })
        .collect();

    if methods.is_empty() {
        return;
    }

    if let Some(&idx) = type_index.get(&type_name) {
        for method in methods {
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

/// Extract methods from a trait impl and attach them to an existing type in the surface.
fn extract_trait_impl_methods(
    item: &syn::ItemImpl,
    crate_name: &str,
    surface: &mut ApiSurface,
    type_index: &AHashMap<String, usize>,
    result_wrapping_aliases: &ahash::AHashSet<String>,
) {
    let type_name = match &*item.self_ty {
        syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    };

    let Some(type_name) = type_name else { return };

    let Some(&idx) = type_index.get(&type_name) else {
        if let Some((_, path, _)) = &item.trait_ {
            if path.segments.last().is_some_and(|s| s.ident == "Default") {
                if let Some(enum_def) = surface.enums.iter_mut().find(|e| e.name == type_name) {
                    enum_def.has_default = true;
                }
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
    let trait_source = item.trait_.as_ref().and_then(|(_, path, _)| {
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

    let type_def = &mut surface.types[idx];

    if let Some((_, path, _)) = &item.trait_ {
        if path.segments.last().is_some_and(|s| s.ident == "Default") {
            type_def.has_default = true;
            extract_default_values(item, &mut type_def.fields);
        }
    }

    let is_conversion_trait = item.trait_.as_ref().is_some_and(|(_, path, _)| {
        path.segments
            .last()
            .is_some_and(|s| matches!(s.ident.to_string().as_str(), "From" | "Into" | "TryFrom" | "TryInto"))
    });
    if is_conversion_trait {
        return;
    }

    let is_std_trait_impl = item.trait_.as_ref().is_some_and(|(_, path, _)| {
        path.segments
            .last()
            .is_some_and(|s| STD_TRAITS.contains(&s.ident.to_string().as_str()))
    });

    for impl_item in &item.items {
        if let syn::ImplItem::Fn(method) = impl_item {
            if !method.sig.generics.params.is_empty() {
                if !is_std_trait_impl && extract_binding_exclusion_reason(&method.attrs).is_none() {
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
            );
            if !type_def.methods.iter().any(|m| m.name == method_def.name) {
                type_def.methods.push(method_def);
            }
        }
    }
}
