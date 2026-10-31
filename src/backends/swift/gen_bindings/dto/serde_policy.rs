use std::collections::HashSet;

use crate::core::ir::TypeDef;

pub(super) fn validate(ty: &TypeDef, exclude_fields: &HashSet<String>) -> anyhow::Result<()> {
    let policy = &ty.serde_deserialization;
    if !policy.deny_unknown_fields && policy.field_aliases.is_empty() {
        return Ok(());
    }
    if !policy.unsupported_field_attributes.is_empty() {
        anyhow::bail!(
            "Swift DTO '{}' cannot preserve serde(deny_unknown_fields) with unsupported {} attributes",
            ty.name,
            policy.unsupported_field_attributes.join(", ")
        );
    }
    if ty.serde_container_conversion.is_present() {
        anyhow::bail!(
            "Swift DTO '{}' has an unsupported strict serde container conversion",
            ty.name
        );
    }
    if ty.has_private_fields
        || ty.fields.iter().any(|field| {
            field.binding_excluded
                || exclude_fields.contains(&field.name)
                || exclude_fields.contains(&format!("{}.{}", ty.name, field.name))
        })
    {
        anyhow::bail!(
            "Swift DTO '{}' cannot preserve strict keys with unexposed fields",
            ty.name
        );
    }
    if let Some(field) = ty.fields.iter().find(|field| field.serde_flatten || field.serde_skip) {
        let attribute = if field.serde_flatten { "flatten" } else { "skip" };
        anyhow::bail!(
            "Swift DTO '{}' has an unsupported strict serde({attribute}) field",
            ty.name
        );
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
