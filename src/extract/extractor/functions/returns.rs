use crate::core::ir::TypeRef;

use crate::extract::type_resolver;

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

    if let syn::Type::Path(type_path) = ty.as_ref() {
        if let Some(seg) = type_path.path.segments.last() {
            let ident = seg.ident.to_string();
            match ident.as_str() {
                "BoxFuture" | "BoxStream" => {
                    let result = extract_future_inner_type(seg)?;
                    if result.1.is_none() && result_wrapping_aliases.contains(&ident) {
                        return Some((result.0, Some("Error".to_string())));
                    }
                    return Some(result);
                }
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
    if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
        for arg in &args.args {
            if let syn::GenericArgument::Type(syn::Type::Path(inner_path)) = arg {
                if let Some(inner_seg) = inner_path.path.segments.last() {
                    if inner_seg.ident == "Box" {
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
            let unwrapped = unwrap_smart_pointer(inner_ty);
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
pub(super) fn detect_cow_return(output: &syn::ReturnType) -> bool {
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
                                syn::Type::Path(p) => {
                                    if let Some(seg) = p.path.segments.last() {
                                        return seg.ident != "str";
                                    }
                                }
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
