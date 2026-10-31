use std::collections::HashSet;

use heck::ToSnakeCase;

use crate::core::ir::TypeDef;

pub(super) fn validate(ty: &TypeDef, exclude_fields: &HashSet<String>) -> anyhow::Result<()> {
    let policy = &ty.serde_deserialization;
    let has_skipped_fields = ty.fields.iter().any(|field| field.serde_skip);
    if !policy.deny_unknown_fields && policy.field_aliases.is_empty() && !has_skipped_fields {
        return Ok(());
    }
    let unsupported: Vec<_> = policy
        .unsupported_field_attributes
        .iter()
        .filter(|attribute| attribute.as_str() != "skip" || !has_skipped_fields)
        .cloned()
        .collect();
    if !unsupported.is_empty() {
        anyhow::bail!(
            "Swift DTO '{}' cannot preserve serde(deny_unknown_fields) with unsupported {} attributes",
            ty.name,
            unsupported.join(", ")
        );
    }
    if ty.serde_container_conversion.is_present() {
        anyhow::bail!(
            "Swift DTO '{}' has an unsupported strict serde container conversion",
            ty.name
        );
    }
    validate_exposure(ty, exclude_fields)?;
    if ty.fields.iter().any(|field| field.serde_flatten) {
        anyhow::bail!("Swift DTO '{}' has an unsupported strict serde(flatten) field", ty.name);
    }
    Ok(())
}

fn validate_exposure(ty: &TypeDef, exclude_fields: &HashSet<String>) -> anyhow::Result<()> {
    if ty.has_private_fields
        || ty.fields.iter().any(|field| {
            !field.serde_skip
                && (field.binding_excluded
                    || exclude_fields.contains(&field.name)
                    || exclude_fields.contains(&format!("{}.{}", ty.name, field.name.to_snake_case())))
        })
    {
        anyhow::bail!(
            "Swift DTO '{}' cannot preserve strict keys with unexposed fields",
            ty.name
        );
    }
    if ty.fields.iter().all(|field| field.serde_skip || field.binding_excluded) {
        anyhow::bail!("Swift DTO '{}' has no representable Codable wire fields", ty.name);
    }
    Ok(())
}

pub(super) fn key_name(public_names: &HashSet<String>) -> String {
    let mut name = "AlefWireKey".to_owned();
    let mut suffix = 0;
    while public_names.contains(&name) {
        suffix += 1;
        name = format!("AlefWireKey{suffix}");
    }
    name
}

pub(super) fn key_type(name: &str) -> String {
    crate::backends::swift::template_env::render(
        "swift_strict_wire_key.swift.jinja",
        crate::alef_context! { wire_key_type => name },
    )
}
