use alef_core::ir::{ParamDef, PrimitiveType, TypeRef};

/// Maps an IR `TypeRef` to the FRB-friendly Rust type string.
/// FRB's primitive ABI: all integers → `i64`, floats → `f64`, strings → `String`.
///
/// NOTE: u64 values above i64::MAX silently wrap to negative on the Dart side
/// because Dart's native `int` is 64-bit signed. Producers of u64-bearing APIs
/// who need the full range should pre-truncate or document the contract.
pub(crate) fn frb_rust_type(ty: &TypeRef, optional: bool) -> String {
    let inner = frb_rust_type_inner(ty);
    if optional { format!("Option<{inner}>") } else { inner }
}

pub(crate) fn frb_rust_type_inner(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::F32 | PrimitiveType::F64 => "f64".to_string(),
            // All integer widths → i64 (Dart int ↔ Rust i64 FRB primitive ABI)
            _ => "i64".to_string(),
        },
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Optional(inner) => format!("Option<{}>", frb_rust_type_inner(inner)),
        TypeRef::Vec(inner) => format!("Vec<{}>", frb_rust_type_inner(inner)),
        TypeRef::Map(k, v) => {
            format!("std::collections::HashMap<{}, {}>", frb_rust_type_inner(k), frb_rust_type_inner(v))
        }
        TypeRef::Named(name) => name.clone(),
        TypeRef::Path => "String".to_string(),
        TypeRef::Unit => "()".to_string(),
        TypeRef::Json => "String".to_string(),
        TypeRef::Duration => "i64".to_string(),
    }
}

pub(crate) fn primitive_name(prim: &PrimitiveType) -> &'static str {
    match prim {
        PrimitiveType::Bool => "bool",
        PrimitiveType::U8 => "u8",
        PrimitiveType::I8 => "i8",
        PrimitiveType::U16 => "u16",
        PrimitiveType::I16 => "i16",
        PrimitiveType::U32 => "u32",
        PrimitiveType::I32 => "i32",
        PrimitiveType::U64 => "u64",
        PrimitiveType::I64 => "i64",
        PrimitiveType::Usize => "usize",
        PrimitiveType::Isize => "isize",
        PrimitiveType::F32 => "f32",
        PrimitiveType::F64 => "f64",
    }
}

/// Like `frb_rust_type`, but Named types resolve to their source-crate path
/// so the bridge fn signature uses the original Rust type (the mirror struct
/// is layout-identical via `#[frb(mirror(T))]`).
pub(crate) fn frb_rust_type_with_source(
    ty: &TypeRef,
    optional: bool,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
) -> String {
    let inner = frb_rust_type_inner_with_source(ty, source_crate, type_paths);
    if optional { format!("Option<{inner}>") } else { inner }
}

pub(crate) fn frb_rust_type_inner_with_source(
    ty: &TypeRef,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
) -> String {
    match ty {
        TypeRef::Named(name) => match type_paths.get(name) {
            Some(path) => path.clone(),
            None => format!("{source_crate}::{name}"),
        },
        TypeRef::Optional(inner) => {
            format!("Option<{}>", frb_rust_type_inner_with_source(inner, source_crate, type_paths))
        }
        TypeRef::Vec(inner) => {
            format!("Vec<{}>", frb_rust_type_inner_with_source(inner, source_crate, type_paths))
        }
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            frb_rust_type_inner_with_source(k, source_crate, type_paths),
            frb_rust_type_inner_with_source(v, source_crate, type_paths)
        ),
        _ => frb_rust_type_inner(ty),
    }
}

/// Build the call-site expression for a single parameter. The IR's
/// `original_type` field carries the Rust type the source function expects;
/// when present, we cast/convert the FRB-widened parameter to match.
pub(crate) fn dart_call_arg(p: &ParamDef) -> String {
    let name = &p.name;
    let original = p.original_type.as_deref().unwrap_or("");
    // Strip outer `&`, `&mut`, leading whitespace.
    let stripped_orig = original
        .trim()
        .trim_start_matches('&')
        .trim_start_matches("mut ")
        .trim();

    // Tuple parameters: the IR flattens `Vec<(A, B, ...)>` to `Vec<String>`,
    // losing the structural shape. Use `original_type` to spot these and emit
    // adapter logic that reconstructs a sensible default tuple. The IR stores
    // `original_type` as Rust Debug syntax (e.g.
    // `Vec(Named("(PathBuf, Option<FileExtractionConfig>)"))`).
    if !stripped_orig.is_empty() && stripped_orig.starts_with("Vec(") && stripped_orig.contains("Named(\"(") {
        let tuple_inner = stripped_orig
            .find("Named(\"(")
            .and_then(|start| {
                let rest = &stripped_orig[start + 8..]; // past `Named("(`
                rest.find(")\")")
                    .map(|end| rest[..end].trim_end_matches(')').to_string())
            })
            .unwrap_or_default();
        if tuple_inner.starts_with("PathBuf,") || tuple_inner.starts_with("PathBuf ,") {
            return format!(
                "{name}.into_iter().map(|p| (std::path::PathBuf::from(p), None)).collect::<Vec<_>>()"
            );
        }
        if tuple_inner.starts_with("Vec<u8>,") || tuple_inner.starts_with("Vec<u8> ,") {
            return format!(
                "{{ let _ = {name}; ::std::unimplemented!(\"batch_extract_bytes from Dart not yet bridged\") }}"
            );
        }
    }

    // Path: FRB sends String; the source likely wants &Path, PathBuf, or
    // Option<PathBuf>. Emit the conversion that matches `is_ref` and `optional`.
    if matches!(p.ty, TypeRef::Path) {
        if p.optional {
            if p.is_ref {
                return format!("{name}.as_ref().map(std::path::Path::new)");
            }
            return format!("{name}.map(std::path::PathBuf::from)");
        }
        if p.is_ref {
            return format!("std::path::Path::new(&{name})");
        }
        return format!("std::path::PathBuf::from({name})");
    }

    // Primitives: FRB widens all integer params to i64 and floats to f64. Cast
    // back to the actual primitive width before forwarding to the source fn.
    if let TypeRef::Primitive(prim) = &p.ty {
        let target = primitive_name(prim);
        if target != "i64" && target != "f64" && target != "bool" {
            if p.optional {
                return format!("{name}.map(|v| v as {target})");
            }
            return if p.is_ref {
                format!("&({name} as {target})")
            } else {
                format!("{name} as {target}")
            };
        }
    }

    // Inner-Vec primitive cast: FRB widens Vec<f32> → Vec<f64>; if the source
    // takes &[f32] we need to materialize a temporary cast Vec.
    if let TypeRef::Vec(inner) = &p.ty {
        if let TypeRef::Primitive(prim) = inner.as_ref() {
            let target = primitive_name(prim);
            if target != "i64" && target != "f64" && target != "bool" {
                if p.optional {
                    if p.is_ref {
                        return format!(
                            "{name}.as_ref().map(|v| v.iter().map(|x| *x as {target}).collect::<Vec<_>>()).as_deref()"
                        );
                    }
                    return format!(
                        "{name}.map(|v| v.into_iter().map(|x| x as {target}).collect::<Vec<_>>())"
                    );
                }
                if p.is_ref {
                    return format!(
                        "{name}.iter().map(|x| *x as {target}).collect::<Vec<_>>().as_slice()"
                    );
                }
                return format!("{name}.into_iter().map(|x| x as {target}).collect::<Vec<_>>()");
            }
        }
    }

    if !p.is_ref {
        return name.clone();
    }

    match (&p.ty, p.optional) {
        (TypeRef::Bytes, false) => format!("&{name}"),
        (TypeRef::String | TypeRef::Char, false) => format!("&{name}"),
        (TypeRef::String | TypeRef::Char, true) => format!("{name}.as_deref()"),
        (TypeRef::Vec(_), true) => format!("{name}.as_deref()"),
        _ => format!("&{name}"),
    }
}
