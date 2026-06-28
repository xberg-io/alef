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
    // itself — when present, no methods from this impl reach the binding surface, for any
    // backend. This is the authoring contract for hiding builder/fluent methods whose
    // names collide with struct fields (e.g. `fn strict(self, on: bool) -> Self` next to
    // a `pub strict: Option<bool>` field), which would otherwise produce duplicate symbols
    // in field-accessor-emitting backends like the C FFI.
    if extract_binding_exclusion_reason(&item.attrs).is_some() {
        return;
    }

    if item.trait_.is_some() {
        // Extract trait impl methods and attach to the type if it's in our surface
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

    // Opaque types expose no public fields, so no field-based constructor is generated for them —
    // a hand-written `new` returning `Self` is their only constructor and must be preserved. For
    // field-based (data-class) types the derived field constructor supersedes such a `new`, so it
    // is dropped (below). Unknown types are treated as opaque to keep the constructor.
    // Enums are always treated as opaque since they have no field-based constructor.
    //
    // A constructor on a *generic* impl block (e.g. `impl<T> ValueDependency<T>`) cannot be lowered
    // to a concrete binding — there is no single `T` — so such constructors are never preserved.
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
                    // `test_config()`); they do not exist in normal builds and would
                    // produce bindings that fail to compile.
                    if is_test_gated(&method.attrs) {
                        return None;
                    }
                    // Skip generic methods — they can't be directly exposed to FFI
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
                    // Skip underscore-prefixed methods — the Rust convention for
                    // "public but not part of the supported API surface" (e.g.
                    // `_testing_*` helpers gated behind test-only cfg features).
                    // These must never reach generated bindings or docs.
                    if method_name.starts_with('_') {
                        return None;
                    }
                    // Skip methods named "new" that return Self for field-based types — the
                    // constructor is already generated from fields. Opaque types have no field
                    // constructor, so their `new` must be preserved as the constructor.
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

    // Use index for O(1) lookup; if not found, check errors and skip enums
    if let Some(&idx) = type_index.get(&type_name) {
        // Dedup: skip methods whose name already exists on the type
        for method in methods {
            if !surface.types[idx].methods.iter().any(|m| m.name == method.name) {
                surface.types[idx].methods.push(method);
            }
        }
    } else if let Some(error_def) = surface.errors.iter_mut().find(|e| e.name == type_name) {
        // This is an impl block on a thiserror error enum. Populate ErrorDef.methods with
        // a fixed whitelist of introspection methods that are safe to expose across the FFI:
        //   - status_code  → maps the error variant to an HTTP status code
        //   - is_transient → indicates whether the error is retryable
        //   - error_type   → returns a &'static str identifier for the error class
        //
        // All other methods (Display helpers, internal utilities, trait impls) are excluded.
        // The whitelist prevents accidentally exporting Rust-only ergonomics to bindings.
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
        // Guard against module-path mismatches: when two enums share the same short name but
        // live in different modules (e.g. `types::ContentPart` vs `realtime::ContentPart`), the
        // one in the API surface has an explicit `rust_path`. Only attach the impl block's
        // static methods when the impl's module path is consistent with the enum's rust_path.
        //
        // `module_path` is a relative path (no crate prefix), derived from the source file:
        //   crates/sample-core/src/types/common.rs  -> "types::common"
        //   crates/sample-core/src/realtime/mod.rs  -> "realtime"
        //
        // `e.rust_path` is fully-qualified with the crate name:
        //   "sample_core::types::ContentPart"
        //
        // Strip the crate prefix ("sample_core::") from rust_path, then extract the module
        // component (everything before the last "::TypeName").
        //
        // Accept if the relative enum module and the impl module_path share a common ancestor
        // (one is a prefix of the other), i.e. they live in the same or adjacent sub-modules.
        let crate_prefix = format!("{crate_name}::");
        let rel = e.rust_path.strip_prefix(&*crate_prefix).unwrap_or(e.rust_path.as_str());
        // rel is now e.g. "types::ContentPart"
        let enum_module_rel = rel.rfind("::").map(|i| &rel[..i]).unwrap_or("");
        if enum_module_rel.is_empty() {
            // The enum is defined at the crate root — accept from any file.
            return true;
        }
        if module_path.is_empty() {
            // The impl block is in the crate root file (lib.rs); the enum is in a sub-module.
            // Only accept if the enum also lives at the root (handled above).
            return false;
        }
        // Accept when the impl's module_path is a prefix of (or equal to) the enum's relative
        // module, or vice versa. Examples:
        //   module_path="types::common", enum_module_rel="types" → "types::common" starts with "types" ✓
        //   module_path="realtime",      enum_module_rel="types" → neither is a prefix ✗
        enum_module_rel.starts_with(module_path) || module_path.starts_with(enum_module_rel)
    }) {
        // This is an impl block on a regular enum (not an error enum).
        // Instance methods on enums are not expressible across most FFI boundaries and are
        // excluded (the original skip behavior is preserved for those).
        // However, associated functions — static factory methods with no `self` receiver —
        // ARE expressible: they act as named constructors that return the enum type, and every
        // binding backend can emit them as static/class-level factory methods.
        // Collect only the associated functions (is_static == true) from this impl block.
        for method in &methods {
            if method.is_static && !enum_def.methods.iter().any(|m| m.name == method.name) {
                enum_def.methods.push(method.clone());
            }
        }
    } else {
        // The impl is for a type we haven't seen as a `pub` struct — create an opaque
        // entry, but flag it `binding_excluded` because the struct's own visibility
        // is unverified (the pub-only first-pass struct extractor at
        // `extract/extractor/mod.rs` rejected it). The common case is a `pub(crate)`
        // struct with `pub` methods: rustc allows the methods to be marked `pub`
        // but their effective visibility is capped at `pub(crate)`. Emitting a
        // binding wrapper (`pub struct Foo { pub(crate) inner: this_crate::path::Foo }`)
        // fails to compile with E0603 ("struct is private"), since the binding crate
        // cannot name the wrapped type. Callers that genuinely want such a type
        // surfaced can opt in via an `alef.toml` config entry.
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

    // Use index for O(1) lookup — only attach to types we already know about
    let Some(&idx) = type_index.get(&type_name) else {
        // Not a struct — it may be an enum with a manual `impl Default for Enum`.
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

    // Extract the trait path from `impl TraitPath for Type`
    // Standard library traits that should NOT be imported (always in scope or from std)
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
        "Deserialize", // serde — re-exported, not crate-local
    ];
    let trait_source = item.trait_.as_ref().and_then(|(_, path, _)| {
        let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        let trait_name = segments.last().map(|s| s.as_str()).unwrap_or("");
        // Skip standard library traits — they don't need explicit imports
        if STD_TRAITS.contains(&trait_name) {
            return None;
        }
        if segments.len() == 1 {
            // Single-segment trait: look up its full path from already-extracted trait types
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

    // Detect `impl Default for Type` — mark type as has_default and extract default values
    if let Some((_, path, _)) = &item.trait_ {
        if path.segments.last().is_some_and(|s| s.ident == "Default") {
            type_def.has_default = true;
            extract_default_values(item, &mut type_def.fields);
        }
    }

    // Skip From/Into/TryFrom/TryInto trait method extraction. These conversions
    // reference Rust-only counterpart types (e.g. `impl From<tree_sitter::Point>
    // for Point` references `tree_sitter::Point`, which has no representation in
    // any target language). Emitting them as binding methods produces nonsensical
    // signatures like `Point.from(Point p)` in Java/C# and uncompilable code in
    // napi where the input type is ambiguous between the JsX wrapper and the
    // Rust counterpart.
    //
    // `Default` is intentionally NOT in this list — `Default::default()` is a
    // legitimate preset constructor that we want emitted as `Type.default()` /
    // `Type::default()` in target languages. The `has_default = true` flag set
    // above handles Default-derived field values for builders; method emission
    // from the impl block handles the `default()` factory itself.
    let is_conversion_trait = item.trait_.as_ref().is_some_and(|(_, path, _)| {
        path.segments
            .last()
            .is_some_and(|s| matches!(s.ident.to_string().as_str(), "From" | "Into" | "TryFrom" | "TryInto"))
    });
    if is_conversion_trait {
        return;
    }

    // Skip impl blocks for standard-library traits whose methods are intrinsically
    // generic (Serialize::serialize<S>, Deserialize::deserialize<D>, Hash::hash<H>,
    // PartialEq::eq via blanket, etc.). The generic parameter is on the trait method
    // signature, not the implementor — consumers never call these directly across the
    // FFI boundary; serde/std dispatch them. Flagging them as
    // `unsupported_generic_item` produces noise; the canonical way to "bind" these
    // implementations is to expose the type and let derive macros handle the codegen.
    let is_std_trait_impl = item.trait_.as_ref().is_some_and(|(_, path, _)| {
        path.segments
            .last()
            .is_some_and(|s| STD_TRAITS.contains(&s.ident.to_string().as_str()))
    });

    // Extract methods from the trait impl (trait methods are implicitly pub)
    for impl_item in &item.items {
        if let syn::ImplItem::Fn(method) = impl_item {
            // Skip generic methods — they can't be directly exposed to FFI
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
            // Don't add duplicates
            if !type_def.methods.iter().any(|m| m.name == method_def.name) {
                type_def.methods.push(method_def);
            }
        }
    }
}
