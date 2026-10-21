use crate::codegen::naming::{dart_value_identifier, pascal_to_camel};
use crate::core::ir::{DefaultValue, FieldDef, PrimitiveType, TypeRef};

pub(super) fn literal(field: &FieldDef) -> Option<String> {
    let value = field.typed_default.as_ref()?;
    if field.optional && matches!(value, DefaultValue::Empty) {
        return Some("null".into());
    }
    render(&field.ty, value)
}

fn render(ty: &TypeRef, value: &DefaultValue) -> Option<String> {
    if let TypeRef::Optional(inner) = ty
        && !matches!(value, DefaultValue::None | DefaultValue::Empty)
    {
        return render(inner, value);
    }
    match value {
        DefaultValue::BoolLiteral(value) => Some(value.to_string()),
        DefaultValue::StringLiteral(value) => Some(
            serde_json::to_string(value)
                .ok()?
                .replace('$', "\\$")
                .replace('~', "\\u007e"),
        ),
        DefaultValue::IntLiteral(value) => Some(value.to_string()),
        DefaultValue::FloatLiteral(value) => crate::codegen::shared::float_literal_digits(*value),
        DefaultValue::None => Some("null".into()),
        DefaultValue::EnumVariant(variant) => Some(format!(
            "{}.{}",
            match ty {
                TypeRef::Named(name) => name.as_str(),
                _ => return None,
            },
            dart_value_identifier(&pascal_to_camel(variant))
        )),
        DefaultValue::ListLiteral(items) => {
            let TypeRef::Vec(element) = ty else { return None };
            let values = items
                .iter()
                .map(|item| render(element, item))
                .collect::<Option<Vec<_>>>()?;
            Some(format!("const [{}]", values.join(", ")))
        }
        DefaultValue::Empty => Some(
            match ty {
                TypeRef::Primitive(PrimitiveType::Bool) => "false",
                TypeRef::Primitive(PrimitiveType::F32 | PrimitiveType::F64) => "0.0",
                TypeRef::Primitive(_) | TypeRef::Duration => "0",
                TypeRef::String | TypeRef::Path => "\"\"",
                TypeRef::Vec(_) | TypeRef::Bytes => "const []",
                TypeRef::Map(_, _) => "const {}",
                TypeRef::Optional(_) => "null",
                _ => return None,
            }
            .into(),
        ),
        _ => None,
    }
}

pub(super) fn frb_literal(field: &FieldDef) -> Option<String> {
    match field.typed_default.as_ref()? {
        DefaultValue::BoolLiteral(value) => Some(value.to_string()),
        DefaultValue::Empty if !field.optional && field.ty == TypeRef::Primitive(PrimitiveType::Bool) => {
            Some("false".into())
        }
        _ => None,
    }
}

pub(super) fn validate(api: &crate::core::ir::ApiSurface) -> anyhow::Result<()> {
    for owner in &api.types {
        for field in crate::codegen::shared::binding_fields(&owner.fields) {
            if owner.is_constructor_extension(field) {
                anyhow::ensure!(
                    literal(field).is_some(),
                    "Dart cannot render constructor default for {}.{} ({:?})",
                    owner.name,
                    field.name,
                    field.ty
                );
            }
        }
    }
    Ok(())
}
