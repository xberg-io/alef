use heck::ToLowerCamelCase;

use crate::backends::swift::naming::swift_source_ident;
use crate::core::ir::{DefaultValue, FieldDef, TypeDef, TypeRef};

use super::{swift_type_based_default, swift_typed_default_literal};

pub(super) fn assignments(ty: &TypeDef, fields: &[&FieldDef]) -> anyhow::Result<String> {
    let mut assignments = String::new();
    for field in fields.iter().filter(|field| field.serde_skip) {
        let expression = default_expression(ty, field).ok_or_else(|| {
            anyhow::anyhow!(
                "Swift DTO '{}.{}' cannot preserve an unproven skipped serde default",
                ty.name,
                field.name
            )
        })?;
        assignments.push_str(&crate::backends::swift::template_env::render(
            "swift_self_assignment.swift.jinja",
            crate::alef_context! {
                field => swift_source_ident(&field.name.to_lower_camel_case()),
                expr => expression,
            },
        ));
    }
    Ok(assignments)
}

fn type_default(field: &FieldDef) -> Option<String> {
    if field.optional || matches!(field.ty, TypeRef::Optional(_)) {
        Some("nil".to_owned())
    } else if field.sanitized || field.newtype_wrapper.is_some() {
        None
    } else {
        swift_type_based_default(&field.ty)
    }
}

fn default_expression(owner: &TypeDef, field: &FieldDef) -> Option<String> {
    if field
        .default
        .as_deref()
        .is_some_and(|value| value.starts_with("serde(default ="))
    {
        return None;
    }
    // Without a container default, serde(skip) uses the field type's Default, not an unrelated owner impl. ~keep
    if field.has_bare_serde_default() || !owner.serde_container_default {
        return type_default(field);
    }
    match field.typed_default.as_ref()? {
        DefaultValue::Empty => type_default(field),
        DefaultValue::None if field.optional || matches!(field.ty, TypeRef::Optional(_)) => Some("nil".to_owned()),
        value => swift_typed_default_literal(value),
    }
}
