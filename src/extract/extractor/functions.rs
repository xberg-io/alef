use crate::core::ir::ApiSurface;
use crate::core::ir::{FunctionDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef, UnsupportedPublicItem};
use ahash::AHashMap;

use crate::extract::type_resolver;

use super::defaults::extract_default_values;
use super::helpers::{
    build_rust_path, extract_binding_exclusion_reason, extract_cfg_condition, extract_doc_comments, unwrap_optional,
};

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
        if (!methods_are_public_by_trait && !super::helpers::is_pub(&method.vis))
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
pub(crate) fn extract_function(item: &syn::ItemFn, crate_name: &str, module_path: &str) -> Option<FunctionDef> {
    // Skip private functions — they cannot be part of the public API surface.
    // This is a defense-in-depth measure to prevent accidental inclusion of
    // internal helper functions even if the visibility check at the call site fails.
    if !super::helpers::is_pub(&item.vis) {
        return None;
    }

    if !item.sig.generics.params.is_empty() {
        return None;
    }

    let binding_exclusion_reason = extract_binding_exclusion_reason(&item.attrs);
    let binding_excluded = binding_exclusion_reason.is_some();
    let cfg = extract_cfg_condition(&item.attrs);
    let name = item.sig.ident.to_string();
    let doc = extract_doc_comments(&item.attrs);
    let mut is_async = item.sig.asyncness.is_some();

    let (mut return_type, mut error_type, returns_ref) = resolve_return_type(&item.sig.output);
    let returns_cow = detect_cow_return(&item.sig.output);

    // Detect future-returning functions as async
    if !is_async {
        let empty = ahash::AHashSet::new();
        if let Some((inner, future_error_type)) = unwrap_future_return(&item.sig.output, &empty) {
            is_async = true;
            return_type = inner;
            // If the future's output is Result<T, E>, propagate the error type.
            if future_error_type.is_some() {
                error_type = future_error_type;
            }
        }
    }

    let params = extract_params(&item.sig.inputs);
    let rust_path = build_rust_path(crate_name, module_path, &name);
    let sanitized = params.iter().any(|p| p.sanitized);

    Some(FunctionDef {
        rust_path,
        original_rust_path: String::new(),
        name,
        params,
        return_type,
        is_async,
        error_type,
        doc,
        cfg,
        sanitized,
        return_sanitized: false,
        returns_ref,
        returns_cow,
        return_newtype_wrapper: None,
        binding_excluded,
        binding_exclusion_reason,
    })
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
                if super::helpers::is_pub(&method.vis) {
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
    } else if surface.enums.iter().any(|e| e.name == type_name) {
        // This is an impl block on a regular enum (not an error enum).
        // Regular enums don't support attached methods in bindings — they exist as pure data
        // with variants only. Methods on enums (like `parse()` helper methods) are skipped.
        // This is the expected behavior: the enum type is already known and emitted, but its
        // impl block methods don't affect the binding surface.
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
        });
    }
}

/// Extract methods from a trait impl and attach them to an existing type in the surface.
pub(crate) fn extract_trait_impl_methods(
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

/// Extract a single method from an impl block.
/// `parent_type_name` is used to resolve `Self` references in return types and params.
/// `trait_source` is the fully qualified trait path if this method comes from a trait impl.
pub(crate) fn extract_method(
    method: &syn::ImplItemFn,
    _crate_name: &str,
    parent_type_name: &str,
    trait_source: Option<String>,
    result_wrapping_aliases: &ahash::AHashSet<String>,
) -> MethodDef {
    let name = method.sig.ident.to_string();
    let doc = extract_doc_comments(&method.attrs);
    let binding_exclusion_reason = extract_binding_exclusion_reason(&method.attrs);
    let binding_excluded = binding_exclusion_reason.is_some();
    let mut is_async = method.sig.asyncness.is_some();

    let (mut return_type, mut error_type, returns_ref) = resolve_return_type(&method.sig.output);

    // Detect if the method returns Cow<'_, T> where T is a named type (not str/bytes).
    // This is used by codegen to emit `.into_owned()` before type conversion.
    let returns_cow = detect_cow_return(&method.sig.output);

    // Detect future-returning functions as async:
    // BoxFuture<'_, T>, Pin<Box<dyn Future<Output = T>>>, etc.
    if !is_async {
        if let Some((inner, future_error_type)) = unwrap_future_return(&method.sig.output, result_wrapping_aliases) {
            is_async = true;
            return_type = inner;
            // If the future's output is Result<T, E>, propagate the error type.
            if future_error_type.is_some() {
                error_type = future_error_type;
            }
        }
    }

    // Resolve `Self` → actual parent type name in return types and params
    resolve_self_refs(&mut return_type, parent_type_name);

    let (receiver, is_static) = detect_receiver(&method.sig.inputs);
    let mut params = extract_params(&method.sig.inputs);
    for param in &mut params {
        resolve_self_refs(&mut param.ty, parent_type_name);
    }

    MethodDef {
        name,
        params,
        return_type,
        is_async,
        is_static,
        error_type,
        doc,
        receiver,
        sanitized: false,
        trait_source,
        returns_ref,
        returns_cow,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded,
        binding_exclusion_reason,
    }
}

/// Replace `TypeRef::Named("Self")` with the actual parent type name, recursively.
fn resolve_self_refs(ty: &mut TypeRef, parent_type_name: &str) {
    match ty {
        TypeRef::Named(n) if n == "Self" => *n = parent_type_name.to_string(),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => resolve_self_refs(inner, parent_type_name),
        TypeRef::Map(k, v) => {
            resolve_self_refs(k, parent_type_name);
            resolve_self_refs(v, parent_type_name);
        }
        _ => {}
    }
}

/// Check if a return type is a future type (BoxFuture, Pin<Box<dyn Future>>, etc.)
/// and extract the inner output type plus optional error type.
///
/// Returns `Some((inner_type, error_type))` where `error_type` is `Some` when the
/// future's output is `Result<T, E>` (i.e. the future wraps a Result).
///
/// `result_wrapping_aliases` contains names of type aliases (e.g. `"BoxFuture"`) whose
/// definition wraps the inner type in `Result<T>`. When the alias is used as
/// `BoxFuture<'_, T>` (T is NOT `Result`), we still mark `is_result=true` because the
/// typedef internally wraps `Result<T>`.
pub(crate) fn unwrap_future_return(
    output: &syn::ReturnType,
    result_wrapping_aliases: &ahash::AHashSet<String>,
) -> Option<(TypeRef, Option<String>)> {
    let ty = match output {
        syn::ReturnType::Type(_, ty) => ty,
        syn::ReturnType::Default => return None,
    };

    // Check the outermost type name
    if let syn::Type::Path(type_path) = ty.as_ref() {
        if let Some(seg) = type_path.path.segments.last() {
            let ident = seg.ident.to_string();
            match ident.as_str() {
                // BoxFuture<'_, T> or BoxStream<'_, T> → async returning T
                "BoxFuture" | "BoxStream" => {
                    let result = extract_future_inner_type(seg)?;
                    // If the alias wraps Result<T> internally and T isn't already Result,
                    // mark as is_result with a generic error type.
                    if result.1.is_none() && result_wrapping_aliases.contains(&ident) {
                        return Some((result.0, Some("Error".to_string())));
                    }
                    return Some(result);
                }
                // Pin<Box<dyn Future<Output = T>>> → async returning T
                "Pin" => {
                    return extract_pin_future_inner(seg);
                }
                _ => {}
            }
        }
    }
    None
}

/// Resolve a syn type that may be `Result<T, E>`, returning `(inner_type, error_type)`.
///
/// If `ty` is `Result<T, E>`, returns `(resolved(T), Some(error_string))`.
/// Otherwise returns `(resolved(ty), None)`.
fn resolve_possibly_result_type(ty: &syn::Type) -> (TypeRef, Option<String>) {
    let error_type = type_resolver::extract_result_error_type(ty);
    let inner = if let Some(unwrapped) = type_resolver::unwrap_result_type(ty) {
        unwrapped
    } else {
        ty
    };
    (type_resolver::resolve_type(inner), error_type)
}

/// Extract inner type from BoxFuture<'_, T> or BoxFuture<'_, Result<T, E>>.
///
/// Returns `(inner_type, error_type)` — `error_type` is `Some` when `T` is `Result<T, E>`.
fn extract_future_inner_type(segment: &syn::PathSegment) -> Option<(TypeRef, Option<String>)> {
    if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
        // BoxFuture has lifetime + type args. Find the type arg (skipping lifetimes).
        for arg in &args.args {
            if let syn::GenericArgument::Type(ty) = arg {
                return Some(resolve_possibly_result_type(ty));
            }
        }
    }
    None
}

/// Extract inner type from Pin<Box<dyn Future<Output = T>>>.
///
/// Returns `(inner_type, error_type)` — `error_type` is `Some` when `Output = Result<T, E>`.
fn extract_pin_future_inner(segment: &syn::PathSegment) -> Option<(TypeRef, Option<String>)> {
    // Pin<Box<dyn Future<Output = T>>>
    if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
        for arg in &args.args {
            if let syn::GenericArgument::Type(syn::Type::Path(inner_path)) = arg {
                if let Some(inner_seg) = inner_path.path.segments.last() {
                    if inner_seg.ident == "Box" {
                        // Box<dyn Future<Output = T>>
                        if let syn::PathArguments::AngleBracketed(box_args) = &inner_seg.arguments {
                            for box_arg in &box_args.args {
                                if let syn::GenericArgument::Type(syn::Type::TraitObject(trait_obj)) = box_arg {
                                    return extract_future_output_from_trait_obj(trait_obj);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Extract Output type from `dyn Future<Output = T>`.
///
/// Returns `(inner_type, error_type)` — `error_type` is `Some` when `Output = Result<T, E>`.
fn extract_future_output_from_trait_obj(trait_obj: &syn::TypeTraitObject) -> Option<(TypeRef, Option<String>)> {
    for bound in &trait_obj.bounds {
        if let syn::TypeParamBound::Trait(trait_bound) = bound {
            if let Some(seg) = trait_bound.path.segments.last() {
                if seg.ident == "Future" {
                    // Look for Output = T in angle-bracketed args
                    if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                        for arg in &args.args {
                            if let syn::GenericArgument::AssocType(assoc) = arg {
                                if assoc.ident == "Output" {
                                    return Some(resolve_possibly_result_type(&assoc.ty));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Detect the receiver kind from method inputs.
pub(crate) fn detect_receiver(
    inputs: &syn::punctuated::Punctuated<syn::FnArg, syn::token::Comma>,
) -> (Option<ReceiverKind>, bool) {
    for input in inputs {
        if let syn::FnArg::Receiver(recv) = input {
            let kind = if recv.reference.is_some() {
                if recv.mutability.is_some() {
                    ReceiverKind::RefMut
                } else {
                    ReceiverKind::Ref
                }
            } else {
                ReceiverKind::Owned
            };
            return (Some(kind), false);
        }
    }
    (None, true)
}

/// Returns `(map_is_ahash, map_key_is_cow, map_is_btree)` for a parameter type.
///
/// Inspects the raw `syn::Type` to detect:
/// - `map_is_ahash`: the outermost (possibly Option-wrapped, possibly &-wrapped) map container
///   is `AHashMap` rather than `HashMap`/`BTreeMap`/etc.
/// - `map_key_is_cow`: the map's first generic argument is `Cow<'_, str>` (or `Cow<'static, str>`).
/// - `map_is_btree`: the outermost map container is `BTreeMap`.
///
/// All flags default to `false` for non-map types.
fn detect_map_metadata(ty: &syn::Type) -> (bool, bool, bool) {
    // Peel Option<...> and &... wrappers to get to the map segment.
    let map_seg = find_map_segment(ty);
    let Some(seg) = map_seg else {
        return (false, false, false);
    };
    let ident = seg.ident.to_string();
    let map_is_ahash = ident == "AHashMap";
    let map_is_btree = ident == "BTreeMap";

    // Check whether the key generic arg is `Cow<...>`.
    let map_key_is_cow = if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
        args.args
            .iter()
            .find_map(|a| {
                if let syn::GenericArgument::Type(syn::Type::Path(tp)) = a {
                    tp.path.segments.last().map(|s| s.ident == "Cow")
                } else {
                    None
                }
            })
            .unwrap_or(false)
    } else {
        false
    };

    (map_is_ahash, map_key_is_cow, map_is_btree)
}

/// Returns `true` when the parameter type (after peeling Option/& wrappers) is `Cow<'_, str>`.
///
/// Used to set `ParamDef::core_wrapper = CoreWrapper::Cow` so call-site codegen can insert
/// `.into()` / `.map(std::borrow::Cow::Owned)` when passing `String` to a `Cow<str>` parameter.
fn param_is_cow_str(ty: &syn::Type) -> bool {
    // Peel Option<...> and &... wrappers.
    let inner = peel_option_and_ref(ty);
    if let syn::Type::Path(tp) = inner {
        if let Some(seg) = tp.path.segments.last() {
            if seg.ident != "Cow" {
                return false;
            }
            // Check that the type arg (after the lifetime) is `str`.
            if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                return args.args.iter().any(|a| {
                    if let syn::GenericArgument::Type(syn::Type::Path(p)) = a {
                        p.path.segments.last().map(|s| s.ident == "str").unwrap_or(false)
                    } else {
                        false
                    }
                });
            }
        }
    }
    false
}

/// Peel one level of `Option<...>` or `&...` from a `syn::Type`.
fn peel_option_and_ref(ty: &syn::Type) -> &syn::Type {
    match ty {
        syn::Type::Reference(r) => r.elem.as_ref(),
        syn::Type::Path(tp) => {
            if let Some(seg) = tp.path.segments.last() {
                if seg.ident == "Option" {
                    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
                        for arg in &ab.args {
                            if let syn::GenericArgument::Type(inner) = arg {
                                return inner;
                            }
                        }
                    }
                }
            }
            ty
        }
        _ => ty,
    }
}

/// Recursively peel `Option<...>`, `&...`, and `Box<...>` wrappers until we reach a
/// `HashMap`/`AHashMap`/`BTreeMap`/etc. segment, or return `None`.
fn find_map_segment(ty: &syn::Type) -> Option<&syn::PathSegment> {
    match ty {
        syn::Type::Reference(r) => find_map_segment(&r.elem),
        syn::Type::Path(tp) => {
            let seg = tp.path.segments.last()?;
            let name = seg.ident.to_string();
            match name.as_str() {
                "HashMap" | "BTreeMap" | "AHashMap" | "IndexMap" | "FxHashMap" => Some(seg),
                "Option" | "Box" | "Arc" | "Rc" => {
                    // Peel the single generic arg and recurse.
                    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
                        for arg in &ab.args {
                            if let syn::GenericArgument::Type(inner) = arg {
                                return find_map_segment(inner);
                            }
                        }
                    }
                    None
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Returns true when `ty` is `Option<&T>` — i.e., the outer type is `Option` and its
/// single generic argument is a reference (`&str`, `&[u8]`, `&Path`, etc.).
/// Used to set `is_ref = true` on optional params even though `&*pat_type.ty` is not a
/// reference (the outer type is `Option`, not `&`).
fn option_inner_is_ref(ty: &syn::Type) -> bool {
    if let syn::Type::Path(type_path) = ty {
        if let Some(seg) = type_path.path.segments.last() {
            if seg.ident == "Option" {
                if let Some(inner) = type_resolver::extract_single_generic_arg_syn(seg) {
                    return matches!(*inner, syn::Type::Reference(_));
                }
            }
        }
    }
    false
}

/// Detect `&mut T` or `Option<&mut T>` parameters.
fn is_mut_ref(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Reference(r) => r.mutability.is_some(),
        syn::Type::Path(type_path) => {
            if let Some(seg) = type_path.path.segments.last() {
                if seg.ident == "Option" {
                    if let Some(inner) = type_resolver::extract_single_generic_arg_syn(seg) {
                        if let syn::Type::Reference(r) = &*inner {
                            return r.mutability.is_some();
                        }
                    }
                }
            }
            false
        }
        _ => false,
    }
}

/// Check if a TypeRef is a tuple type.
fn is_tuple_type(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Named(n) => n.starts_with('('),
        TypeRef::Vec(inner) => is_tuple_type(inner),
        TypeRef::Optional(inner) => is_tuple_type(inner),
        _ => false,
    }
}

/// True if `ty` is `&[&T]`, `Vec<&T>`, `Option<&[&T]>`, `Option<Vec<&T>>`, or `&Vec<&T>`.
/// FFI codegen uses this to emit a `Vec<&T>` intermediate when calling the core function
/// (since `&Vec<T>` coerces to `&[T]`, not `&[&T]`).
fn vec_inner_is_ref(ty: &syn::Type) -> bool {
    // Strip outer reference: `&X` -> `X`
    let deref_ty = if let syn::Type::Reference(r) = ty {
        r.elem.as_ref()
    } else {
        ty
    };

    // Strip outer Option: `Option<X>` -> `X`, and check element type
    let to_check = if let syn::Type::Path(type_path) = deref_ty {
        if let Some(seg) = type_path.path.segments.last() {
            if seg.ident == "Option" {
                if let Some(inner) = type_resolver::extract_single_generic_arg_syn(seg) {
                    inner
                } else {
                    return false;
                }
            } else {
                // Not an Option, check the path itself
                Box::new(deref_ty.clone())
            }
        } else {
            return false;
        }
    } else {
        Box::new(deref_ty.clone())
    };

    // Now check if to_check is either:
    // 1. A Slice `[T]` where T is a Reference
    // 2. A Path ending with `Vec<T>` where T is a Reference
    match to_check.as_ref() {
        syn::Type::Slice(slice) => matches!(*slice.elem, syn::Type::Reference(_)),
        syn::Type::Path(type_path) => {
            if let Some(seg) = type_path.path.segments.last() {
                if seg.ident == "Vec" {
                    if let Some(elem_type) = type_resolver::extract_single_generic_arg_syn(seg) {
                        matches!(*elem_type, syn::Type::Reference(_))
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Extract function/method parameters, skipping `self` receivers.
pub(crate) fn extract_params(inputs: &syn::punctuated::Punctuated<syn::FnArg, syn::token::Comma>) -> Vec<ParamDef> {
    inputs
        .iter()
        .filter_map(|arg| {
            if let syn::FnArg::Typed(pat_type) = arg {
                let name = match &*pat_type.pat {
                    syn::Pat::Ident(ident) => ident.ident.to_string(),
                    _ => "_".to_string(),
                };
                // `is_ref` is true for `&T` params AND for `Option<&T>` params.
                // The latter is needed to distinguish `Option<&str>` (core takes &str slice)
                // from `Option<String>` (core takes owned String).
                let is_ref = matches!(&*pat_type.ty, syn::Type::Reference(_)) || option_inner_is_ref(&pat_type.ty);
                let is_mut = is_mut_ref(&pat_type.ty);
                let resolved = type_resolver::resolve_type(&pat_type.ty);

                // Detect AHashMap/BTreeMap container and Cow key before type erasure.
                let (map_is_ahash, map_key_is_cow, map_is_btree) = detect_map_metadata(&pat_type.ty);

                // Detect Cow<str> parameters — the extractor resolves them to TypeRef::String
                // but call-site codegen must insert `.into()` / `.map(Cow::Owned)`.
                let core_wrapper = if param_is_cow_str(&pat_type.ty) {
                    crate::core::ir::CoreWrapper::Cow
                } else {
                    crate::core::ir::CoreWrapper::None
                };

                // Check if the resolved type (before unwrapping optional) is a tuple type
                let sanitized = is_tuple_type(&resolved);

                // Capture original type before sanitization for codegen deserialization
                let original_type = if sanitized {
                    Some(format!("{:?}", resolved))
                } else {
                    None
                };

                let (ty, optional) = unwrap_optional(resolved);
                Some(ParamDef {
                    name,
                    ty,
                    optional,
                    default: None,
                    sanitized,
                    typed_default: None,
                    is_ref,
                    is_mut,
                    newtype_wrapper: None,
                    original_type,
                    map_is_ahash,
                    map_key_is_cow,
                    vec_inner_is_ref: vec_inner_is_ref(&pat_type.ty),
                    map_is_btree,
                    core_wrapper,
                })
            } else {
                None // Skip self receiver
            }
        })
        .collect()
}

/// Resolve the return type, extract error type, and detect reference returns.
///
/// Returns `(resolved_type, error_type, returns_ref)`.
/// `returns_ref` is true when the core return type (after Result unwrapping) is a
/// reference — e.g. `&T`, `Option<&str>`, `&[u8]`. Code generators use this flag
/// to insert `.clone()` before type conversion in delegation code.
pub(crate) fn resolve_return_type(output: &syn::ReturnType) -> (TypeRef, Option<String>, bool) {
    match output {
        syn::ReturnType::Default => (TypeRef::Unit, None, false),
        syn::ReturnType::Type(_, ty) => {
            let error_type = type_resolver::extract_result_error_type(ty);
            let inner_ty = if let Some(inner) = type_resolver::unwrap_result_type(ty) {
                inner
            } else {
                ty.as_ref()
            };
            // Unwrap Box/Arc/Rc wrappers to check the actual inner type
            let unwrapped = unwrap_smart_pointer(inner_ty);
            // Cow<'_, NamedType> returns also need special handling — treat as returns_ref
            // so codegen can emit `.into_owned()` instead of direct `.into()`.
            let returns_ref = syn_type_contains_ref(unwrapped) || is_cow_named_return(inner_ty);
            let resolved = type_resolver::resolve_type(inner_ty);
            (resolved, error_type, returns_ref)
        }
    }
}

/// Unwrap Box<T>, Arc<T>, Rc<T> wrappers to get the inner syn::Type.
fn unwrap_smart_pointer(ty: &syn::Type) -> &syn::Type {
    if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            let ident = segment.ident.to_string();
            if matches!(ident.as_str(), "Box" | "Arc" | "Rc") {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    for arg in &args.args {
                        if let syn::GenericArgument::Type(inner) = arg {
                            return inner;
                        }
                    }
                }
            }
        }
    }
    ty
}

/// Check if a syn::Type is or contains a reference.
///
/// Detects: `&T`, `Option<&T>`, `Vec<&T>`, etc.
fn syn_type_contains_ref(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Reference(_) => true,
        syn::Type::Path(type_path) => {
            if let Some(segment) = type_path.path.segments.last() {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    return args.args.iter().any(|arg| {
                        if let syn::GenericArgument::Type(inner) = arg {
                            syn_type_contains_ref(inner)
                        } else {
                            false
                        }
                    });
                }
            }
            false
        }
        _ => false,
    }
}

/// Check if a method's return type is `Cow<'_, T>` where T is a named type.
fn detect_cow_return(output: &syn::ReturnType) -> bool {
    if let syn::ReturnType::Type(_, ty) = output {
        is_cow_named_return(ty)
    } else {
        false
    }
}

/// Check if a type is `Cow<'_, T>` where T is a named (struct/enum) type.
///
/// Returns true for `Cow<'_, MyStruct>` but false for `Cow<'_, str>` (→ String)
/// or `Cow<'_, [u8]>` (→ Bytes). Used so codegen can emit `.into_owned()`.
fn is_cow_named_return(ty: &syn::Type) -> bool {
    if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "Cow" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    for arg in &args.args {
                        if let syn::GenericArgument::Type(inner) = arg {
                            match inner {
                                // Cow<'_, str> → maps to String naturally, not a "cow named return"
                                syn::Type::Path(p) => {
                                    if let Some(seg) = p.path.segments.last() {
                                        return seg.ident != "str";
                                    }
                                }
                                // Cow<'_, [u8]> → maps to Bytes naturally
                                syn::Type::Slice(_) => return false,
                                _ => return true,
                            }
                        }
                    }
                }
            }
        }
    }
    false
}
