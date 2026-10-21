use crate::codegen::java_literal::escape_java_string_literal;
use crate::codegen::naming::{PublicIdentifierKind, public_host_identifier};
use crate::core::config::Language;
use crate::core::ir::{DefaultValue, FieldDef, PrimitiveType, TypeRef};

pub(super) fn java_constructor_default(field: &FieldDef) -> anyhow::Result<String> {
    let value = field
        .typed_default
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Java constructor extension {} has no resolved default", field.name))?;
    if field.optional && matches!(value, DefaultValue::Empty | DefaultValue::None) {
        return Ok("null".into());
    }
    literal(&field.ty, value).ok_or_else(|| {
        anyhow::anyhow!(
            "Java cannot render constructor default for {} ({:?})",
            field.name,
            field.ty
        )
    })
}

fn literal(ty: &TypeRef, value: &DefaultValue) -> Option<String> {
    if let TypeRef::Optional(inner) = ty
        && !matches!(value, DefaultValue::None | DefaultValue::Empty)
    {
        return literal(inner, value);
    }
    if let Some(value) = super::super::helpers::java_literal_default(ty, Some(value)) {
        return Some(value);
    }
    match value {
        DefaultValue::BoolLiteral(value) => Some(value.to_string()),
        DefaultValue::IntLiteral(value) => Some(match ty {
            TypeRef::Primitive(
                PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize,
            )
            | TypeRef::Duration => format!("{value}L"),
            _ => value.to_string(),
        }),
        DefaultValue::FloatLiteral(value) => {
            let digits = crate::codegen::shared::float_literal_digits(*value)?;
            Some(if matches!(ty, TypeRef::Primitive(PrimitiveType::F32)) {
                format!("{digits}f")
            } else {
                digits
            })
        }
        DefaultValue::StringLiteral(value) => Some(format!(
            "\"{}\"",
            escape_java_string_literal(value).replace('~', "\\u007e")
        )),
        DefaultValue::EnumVariant(variant) => Some(format!(
            "{}.{}",
            match ty {
                TypeRef::Named(name) => name.as_str(),
                _ => return None,
            },
            public_host_identifier(Language::Java, PublicIdentifierKind::EnumVariant, variant)
        )),
        DefaultValue::ListLiteral(items) => {
            let TypeRef::Vec(element) = ty else { return None };
            let values = items
                .iter()
                .map(|value| literal(element, value))
                .collect::<Option<Vec<_>>>()?;
            Some(format!("java.util.List.of({})", values.join(", ")))
        }
        DefaultValue::Empty => empty_literal(ty),
        DefaultValue::None => Some("null".into()),
        _ => None,
    }
}

fn empty_literal(ty: &TypeRef) -> Option<String> {
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => literal(ty, &DefaultValue::BoolLiteral(false)),
        TypeRef::Primitive(PrimitiveType::F32 | PrimitiveType::F64) => literal(ty, &DefaultValue::FloatLiteral(0.0)),
        TypeRef::Primitive(_) | TypeRef::Duration => literal(ty, &DefaultValue::IntLiteral(0)),
        TypeRef::Bytes => Some("new byte[0]".into()),
        _ => Some(crate::codegen::config_gen::default_value_for_field(
            &FieldDef {
                ty: ty.clone(),
                typed_default: Some(DefaultValue::Empty),
                ..Default::default()
            },
            "java",
        )),
    }
}
