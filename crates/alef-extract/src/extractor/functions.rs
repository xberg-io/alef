use ahash::AHashMap;
use alef_core::ir::ApiSurface;
use alef_core::ir::{FunctionDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
use syn;

use crate::type_resolver;

use super::defaults::extract_default_values;
use super::helpers::{build_rust_path, extract_cfg_condition, extract_doc_comments, unwrap_optional};

/// Extract a public free function into a `FunctionDef`.
/// Returns `None` for generic functions — they can't be directly exposed to FFI.
pub(crate) fn extract_function(item: &syn::ItemFn, crate_name: &str, module_path: &str) -> Option<FunctionDef> {
    if !item.sig.generics.params.is_empty() {
        return None;
    }
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

    Some(FunctionDef {
        rust_path,
        name,
        params,
        return_type,
        is_async,
        error_type,
        doc,
        cfg,
        sanitized: false,
        returns_ref,
        returns_cow,
        return_newtype_wrapper: None,
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

    let methods: Vec<MethodDef> = item
        .items
        .iter()
        .filter_map(|impl_item| {
            if let syn::ImplItem::Fn(method) = impl_item {
                if super::helpers::is_pub(&method.vis) {
                    // Skip generic methods — they can't be directly exposed to FFI
                    if !method.sig.generics.params.is_empty() {
                        return None;
                    }
                    // Skip methods named "new" that return Self — constructor already generated from fields
                    let method_name = method.sig.ident.to_string();
                    if method_name == "new" {
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

    // Use index for O(1) lookup; if not found, create opaque type
    if let Some(&idx) = type_index.get(&type_name) {
        // Dedup: skip methods whose name already exists on the type
        for method in methods {
            if !surface.types[idx].methods.iter().any(|m| m.name == method.name) {
                surface.types[idx].methods.push(method);
            }
        }
    } else {
        // The impl is for a type we haven't seen as a pub struct — create an opaque entry
        let rust_path = build_rust_path(crate_name, module_path, &type_name);
        surface.types.push(TypeDef {
            name: type_name.clone(),
            rust_path,
            fields: vec![],
            methods,
            is_opaque: true,
            is_clone: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            doc: String::new(),
            cfg: None,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
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

    // Extract methods from the trait impl (trait methods are implicitly pub)
    for impl_item in &item.items {
        if let syn::ImplItem::Fn(method) = impl_item {
            // Skip generic methods — they can't be directly exposed to FFI
            if !method.sig.generics.params.is_empty() {
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
                let (ty, optional) = unwrap_optional(resolved);
                Some(ParamDef {
                    name,
                    ty,
                    optional,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref,
                    is_mut,
                    newtype_wrapper: None,
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
