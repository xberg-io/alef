//! Parameter type formatting and serde recovery call argument helpers.

use crate::codegen::generators;
use crate::core::ir::TypeRef;
use ahash::AHashSet;

pub(super) fn typeref_to_core_type_str(ty: &TypeRef) -> String {
    use crate::core::ir::PrimitiveType;
    match ty {
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::U8 => "u8".to_string(),
            PrimitiveType::U16 => "u16".to_string(),
            PrimitiveType::U32 => "u32".to_string(),
            PrimitiveType::U64 => "u64".to_string(),
            PrimitiveType::I8 => "i8".to_string(),
            PrimitiveType::I16 => "i16".to_string(),
            PrimitiveType::I32 => "i32".to_string(),
            PrimitiveType::I64 => "i64".to_string(),
            PrimitiveType::F32 => "f32".to_string(),
            PrimitiveType::F64 => "f64".to_string(),
            PrimitiveType::Usize => "usize".to_string(),
            PrimitiveType::Isize => "isize".to_string(),
        },
        TypeRef::Vec(inner) => format!("Vec<{}>", typeref_to_core_type_str(inner)),
        TypeRef::Optional(inner) => format!("Option<{}>", typeref_to_core_type_str(inner)),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            typeref_to_core_type_str(k),
            typeref_to_core_type_str(v)
        ),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Path => "String".to_string(),
        TypeRef::Duration => "u64".to_string(),
        TypeRef::Named(n) => n.to_string(),
        TypeRef::Unit => "()".to_string(),
    }
}

/// Helper: format a parameter, prefixing with _ if unused
pub(in crate::backends::wasm::gen_bindings) fn format_param_unused(name: &str, ty: &str, unused: bool) -> String {
    let prefix = if unused { "_" } else { "" };
    format!("{}{}: {}", prefix, name, ty)
}

/// Returns a type name in turbofish form for use before `::from(expr)`.
///
/// Rust requires turbofish when a type has generic parameters and sits before `::`:
///   `Vec<T>::from(x)` is a syntax error — `Vec::<T>::from(x)` is required.
pub(super) fn wasm_serde_recovery_call_args(
    params: &[crate::core::ir::ParamDef],
    opaque_types: &AHashSet<String>,
) -> String {
    params
        .iter()
        .map(|p| match &p.ty {
            TypeRef::Vec(inner)
                if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref && !p.optional =>
            {
                format!("&{}_refs", p.name)
            }
            _ => generators::gen_call_args_with_let_bindings(std::slice::from_ref(p), opaque_types),
        })
        .collect::<Vec<_>>()
        .join(", ")
}
