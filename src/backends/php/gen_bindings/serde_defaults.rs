use crate::codegen::naming::pascal_to_snake;
use crate::core::ir::{ApiSurface, DefaultValue, PrimitiveType, TypeRef};

fn serde_default_fn_name(type_name: &str, field_name: &str) -> String {
    format!("{}_{}", pascal_to_snake(type_name), pascal_to_snake(field_name))
}

fn typed_default_fn(default: &DefaultValue, ty: &TypeRef) -> Option<(&'static str, String)> {
    match (default, ty) {
        (DefaultValue::BoolLiteral(value), TypeRef::Primitive(PrimitiveType::Bool)) => {
            Some(("bool", value.to_string()))
        }
        // Only emit String-returning fn for actual String fields. Named (enum-backed struct
        // wrapper) fields would mismatch the wrapped Named return type at compile time, so
        // skip emission and let serde fall back to Default for the wrapped type.
        (DefaultValue::StringLiteral(value) | DefaultValue::EnumVariant(value), TypeRef::String) => {
            Some(("String", format!("{value:?}.to_string()")))
        }
        (DefaultValue::IntLiteral(value), TypeRef::Primitive(primitive)) => {
            let return_type = match primitive {
                PrimitiveType::U8 => "u8",
                PrimitiveType::U16 => "u16",
                PrimitiveType::U32 => "u32",
                PrimitiveType::U64 => "i64",
                PrimitiveType::I8 => "i8",
                PrimitiveType::I16 => "i16",
                PrimitiveType::I32 => "i32",
                PrimitiveType::I64 => "i64",
                PrimitiveType::Usize | PrimitiveType::Isize => "i64",
                PrimitiveType::Bool | PrimitiveType::F32 | PrimitiveType::F64 => return None,
            };
            Some((return_type, value.to_string()))
        }
        (DefaultValue::FloatLiteral(value), TypeRef::Primitive(PrimitiveType::F32)) => {
            let s = format!("{value}");
            let body = if s.contains('.') || s.contains('e') {
                s
            } else {
                format!("{s}.0")
            };
            Some(("f32", body))
        }
        (DefaultValue::FloatLiteral(value), TypeRef::Primitive(PrimitiveType::F64)) => {
            let s = format!("{value}");
            let body = if s.contains('.') || s.contains('e') {
                s
            } else {
                format!("{s}.0")
            };
            Some(("f64", body))
        }
        _ => None,
    }
}

pub(super) fn gen_serde_defaults_module(api: &ApiSurface) -> Option<String> {
    let mut functions = Vec::new();
    for typ in api.types.iter().filter(|typ| typ.has_default) {
        for (field, default) in typ
            .fields
            .iter()
            .filter(|field| !field.optional)
            .filter_map(|field| field.typed_default.as_ref().map(|default| (field, default)))
        {
            let Some((return_type, body)) = typed_default_fn(default, &field.ty) else {
                continue;
            };
            let fn_name = serde_default_fn_name(&typ.name, &field.name);
            functions.push(format!("    pub fn {fn_name}() -> {return_type} {{ {body} }}"));
        }
    }

    if functions.is_empty() {
        None
    } else {
        Some(format!("mod serde_defaults {{\n{}\n}}", functions.join("\n")))
    }
}
