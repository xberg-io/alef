use alef_core::ir::{ParamDef, TypeRef};

use super::conversions::{frb_rust_type_inner, primitive_name};

/// Build the owned (non-ref) Rust type using original widths (not FRB-widened).
pub(super) fn owned_ty(ty: &TypeRef, src: &str, tp: &std::collections::HashMap<String, String>) -> String {
    match ty {
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Path => "std::path::PathBuf".to_string(),
        TypeRef::Primitive(prim) => primitive_name(prim).to_string(),
        TypeRef::Named(name) => match tp.get(name) {
            Some(path) => path.clone(),
            None => format!("{src}::{name}"),
        },
        TypeRef::Vec(inner) => format!("Vec<{}>", owned_ty(inner, src, tp)),
        TypeRef::Optional(inner) => format!("Option<{}>", owned_ty(inner, src, tp)),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            owned_ty(k, src, tp),
            owned_ty(v, src, tp)
        ),
        TypeRef::Unit => "()".to_string(),
        TypeRef::Json => "String".to_string(),
        TypeRef::Duration => "std::time::Duration".to_string(),
    }
}

/// Build the parameter type for the `impl Trait` method signature — must match
/// the original trait exactly (ref, mut-ref, original primitive widths).
pub(super) fn trait_impl_param_type(
    p: &ParamDef,
    source_crate_name: &str,
    type_paths: &std::collections::HashMap<String, String>,
) -> String {
    if p.is_ref {
        // Reference parameters: use the slice/ref form for Bytes/Vec, plain ref for others.
        match &p.ty {
            TypeRef::Bytes => {
                if p.is_mut { "&mut [u8]".to_string() } else { "&[u8]".to_string() }
            }
            TypeRef::Vec(inner) => {
                let inner_ty = match inner.as_ref() {
                    TypeRef::Primitive(prim) => primitive_name(prim).to_string(),
                    TypeRef::Named(name) => match type_paths.get(name) {
                        Some(path) => path.clone(),
                        None => format!("{source_crate_name}::{name}"),
                    },
                    _ => frb_rust_type_inner(inner),
                };
                if p.is_mut { format!("&mut [{inner_ty}]") } else { format!("&[{inner_ty}]") }
            }
            TypeRef::String | TypeRef::Char => {
                if p.is_mut { "&mut String".to_string() } else { "&str".to_string() }
            }
            TypeRef::Path => {
                if p.is_mut { "&mut std::path::Path".to_string() } else { "&std::path::Path".to_string() }
            }
            _ => {
                let base = owned_ty(&p.ty, source_crate_name, type_paths);
                if p.is_mut { format!("&mut {base}") } else { format!("&{base}") }
            }
        }
    } else if p.optional {
        format!("Option<{}>", owned_ty(&p.ty, source_crate_name, type_paths))
    } else {
        owned_ty(&p.ty, source_crate_name, type_paths)
    }
}

/// Build the conversion expression that converts the parameter from its original
/// trait type to the FRB-friendly owned type expected by the DartFnFuture closure.
/// Returns an empty string if no conversion is needed.
pub(super) fn trait_impl_param_conversion(p: &ParamDef) -> String {
    let name = &p.name;
    if p.is_ref {
        match &p.ty {
            TypeRef::Bytes => format!("let {name} = {name}.to_vec();"),
            TypeRef::String | TypeRef::Char => format!("let {name} = {name}.to_string();"),
            TypeRef::Path => format!("let {name} = {name}.to_string_lossy().into_owned();"),
            TypeRef::Vec(inner) => {
                // &[T] → Vec<T>; inner type may need widening
                let orig = match inner.as_ref() {
                    TypeRef::Primitive(prim) => primitive_name(prim).to_string(),
                    _ => return format!("let {name} = {name}.to_vec();"),
                };
                let target = frb_rust_type_inner(inner);
                if target != orig {
                    // e.g. &[f32] → Vec<f64>
                    format!("let {name} = {name}.iter().map(|x| *x as {target}).collect::<Vec<_>>();")
                } else {
                    format!("let {name} = {name}.to_vec();")
                }
            }
            TypeRef::Named(_) => format!("let {name} = {name}.clone();"),
            _ => String::new(),
        }
    } else {
        // Non-ref: primitive widening might be needed for the closure
        if let TypeRef::Primitive(prim) = &p.ty {
            let orig = primitive_name(prim);
            let widened = frb_rust_type_inner(&p.ty);
            if orig != widened {
                return format!("let {name} = {name} as {widened};");
            }
        }
        String::new()
    }
}

/// Build a return-value conversion suffix to transform the FRB-widened return
/// value from the DartFnFuture closure back to the original trait return type.
/// Returns an empty string when no conversion is needed.
pub(super) fn trait_impl_return_conversion(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(prim) => {
            let orig = primitive_name(prim);
            let widened = frb_rust_type_inner(ty);
            if orig != widened { format!(" as {orig}") } else { String::new() }
        }
        TypeRef::Vec(inner) => {
            if let TypeRef::Vec(inner2) = inner.as_ref() {
                // Vec<Vec<f32>> → Vec<Vec<f64>> widened; convert back
                if let TypeRef::Primitive(prim) = inner2.as_ref() {
                    let orig = primitive_name(prim);
                    let widened = frb_rust_type_inner(inner2);
                    if orig != widened {
                        return format!(
                            ".into_iter().map(|v| v.into_iter().map(|x| x as {orig}).collect()).collect::<Vec<_>>()"
                        );
                    }
                }
                return String::new();
            }
            if let TypeRef::Primitive(prim) = inner.as_ref() {
                let orig = primitive_name(prim);
                let widened = frb_rust_type_inner(inner);
                if orig != widened {
                    return format!(".into_iter().map(|x| x as {orig}).collect::<Vec<_>>()");
                }
            }
            String::new()
        }
        _ => String::new(),
    }
}

/// Return the original Rust type for use in the `impl Trait` return position.
/// Uses the original primitive widths (not FRB-widened).
pub(super) fn trait_impl_return_type(
    ty: &TypeRef,
    source_crate_name: &str,
    type_paths: &std::collections::HashMap<String, String>,
) -> String {
    match ty {
        TypeRef::Primitive(prim) => primitive_name(prim).to_string(),
        TypeRef::Unit => "()".to_string(),
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Named(name) => match type_paths.get(name) {
            Some(path) => path.clone(),
            None => format!("{source_crate_name}::{name}"),
        },
        TypeRef::Vec(inner) => format!("Vec<{}>", trait_impl_return_type(inner, source_crate_name, type_paths)),
        TypeRef::Optional(inner) => {
            format!("Option<{}>", trait_impl_return_type(inner, source_crate_name, type_paths))
        }
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            trait_impl_return_type(k, source_crate_name, type_paths),
            trait_impl_return_type(v, source_crate_name, type_paths),
        ),
        TypeRef::Path => "std::path::PathBuf".to_string(),
        TypeRef::Duration => "std::time::Duration".to_string(),
        TypeRef::Json => "String".to_string(),
    }
}
