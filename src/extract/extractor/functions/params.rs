use crate::core::ir::{ParamDef, ReceiverKind, TypeRef};

use crate::extract::type_resolver;

use super::super::helpers::unwrap_optional;

/// Detect the receiver kind from method inputs.
pub(crate) fn detect_receiver(
    inputs: &syn::punctuated::Punctuated<syn::FnArg, syn::token::Comma>,
) -> (Option<ReceiverKind>, bool) {
    for input in inputs {
        if let syn::FnArg::Receiver(recv) = input {
            let kind = match &recv.kind {
                syn::ReceiverKind::Reference(_, _, mutability) => {
                    if mutability.is_some() {
                        ReceiverKind::RefMut
                    } else {
                        ReceiverKind::Ref
                    }
                }
                _ => ReceiverKind::Owned,
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
    let map_seg = find_map_segment(ty);
    let Some(seg) = map_seg else {
        return (false, false, false);
    };
    let ident = seg.ident.to_string();
    let map_is_ahash = ident == "AHashMap";
    let map_is_btree = ident == "BTreeMap";

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
    let inner = peel_option_and_ref(ty);
    if let syn::Type::Path(tp) = inner {
        if let Some(seg) = tp.path.segments.last() {
            if seg.ident != "Cow" {
                return false;
            }
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
    let deref_ty = if let syn::Type::Reference(r) = ty {
        r.elem.as_ref()
    } else {
        ty
    };

    let to_check = if let syn::Type::Path(type_path) = deref_ty {
        if let Some(seg) = type_path.path.segments.last() {
            if seg.ident == "Option" {
                if let Some(inner) = type_resolver::extract_single_generic_arg_syn(seg) {
                    inner
                } else {
                    return false;
                }
            } else {
                Box::new(deref_ty.clone())
            }
        } else {
            return false;
        }
    } else {
        Box::new(deref_ty.clone())
    };

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
                let is_ref = matches!(&*pat_type.ty, syn::Type::Reference(_)) || option_inner_is_ref(&pat_type.ty);
                let is_mut = is_mut_ref(&pat_type.ty);
                let resolved = type_resolver::resolve_type(&pat_type.ty);

                let (map_is_ahash, map_key_is_cow, map_is_btree) = detect_map_metadata(&pat_type.ty);

                let core_wrapper = if param_is_cow_str(&pat_type.ty) {
                    crate::core::ir::CoreWrapper::Cow
                } else {
                    crate::core::ir::CoreWrapper::None
                };

                let sanitized = is_tuple_type(&resolved);

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
                None
            }
        })
        .collect()
}
