use crate::core::ir::{DefaultValue, FieldDef, PrimitiveType, TypeRef};

use super::types::zig_enum_default;

pub(super) fn literal(field: &FieldDef, enums: &[crate::core::ir::EnumDef]) -> Option<String> {
    let value = field.typed_default.as_ref()?;
    if field.optional && matches!(value, DefaultValue::Empty) {
        return Some("null".into());
    }
    render(&field.ty, value, enums)
}

fn render(ty: &TypeRef, value: &DefaultValue, enums: &[crate::core::ir::EnumDef]) -> Option<String> {
    if let TypeRef::Optional(inner) = ty
        && !matches!(value, DefaultValue::None | DefaultValue::Empty)
    {
        return render(inner, value, enums);
    }
    match value {
        DefaultValue::BoolLiteral(value) => Some(value.to_string()),
        DefaultValue::StringLiteral(value) => Some(format!("\"{}\"", escape_zig_string(value))),
        DefaultValue::IntLiteral(value) => Some(value.to_string()),
        DefaultValue::FloatLiteral(value) => crate::codegen::shared::float_literal_digits(*value),
        DefaultValue::None => Some("null".into()),
        DefaultValue::EnumVariant(variant) => {
            let TypeRef::Named(name) = ty else { return None };
            zig_enum_default(enums.iter().find(|en| en.name == *name)?, variant)
        }
        DefaultValue::ListLiteral(items) => {
            let TypeRef::Vec(element) = ty else { return None };
            let values = items
                .iter()
                .map(|item| render(element, item, enums))
                .collect::<Option<Vec<_>>>()?;
            Some(format!("&.{{{}}}", values.join(", ")))
        }
        DefaultValue::Empty => Some(
            match ty {
                TypeRef::Primitive(PrimitiveType::Bool) => "false",
                TypeRef::Primitive(PrimitiveType::F32 | PrimitiveType::F64) => "0.0",
                TypeRef::Primitive(_) | TypeRef::Duration => "0",
                TypeRef::String | TypeRef::Path => "\"\"",
                TypeRef::Vec(_) | TypeRef::Bytes => "&.{}",
                TypeRef::Optional(_) => "null",
                _ => return None,
            }
            .into(),
        ),
        _ => None,
    }
}

fn escape_zig_string(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'"' => "\\\"".into(),
            b'\\' => "\\\\".into(),
            b'\n' => "\\n".into(),
            b'\r' => "\\r".into(),
            b'\t' => "\\t".into(),
            b'~' => "\\x7e".into(),
            b' '..=b'~' => char::from(byte).to_string(),
            _ => format!("\\x{byte:02x}"),
        })
        .collect()
}
